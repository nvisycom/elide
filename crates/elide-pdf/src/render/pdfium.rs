//! The PDFium binding and its dedicated render thread.
//!
//! PDFium is not thread-safe and an open document borrows the binding, so all
//! rendering is serialised on a dedicated single-thread pool; the binding is
//! created once on first use via a `thread_local!` and reused.

use std::cell::RefCell;
use std::sync::LazyLock;

use image::{GenericImageView, ImageFormat};
use pdfium_render::prelude::*;

use super::{Glyph, GlyphSource, PageObservation, PixelRect, RenderedPage};
use crate::error::{Error, Result};

/// Maximum number of pages [`observe_all`](Binding::observe_all) will render.
/// Each page retains a full RGB8 buffer, so an unbounded page count is a memory
/// DoS vector; 10,000 matches the inspection page bound and covers any real
/// document.
const MAX_PAGES: usize = 10_000;

/// Maximum rendered width or height, in pixels, of any single page. A page
/// buffer is `width * height * 3` bytes, so this caps one page near 3 GiB at the
/// extreme (100k x 100k), well beyond any legitimate render, while refusing a
/// malicious page that demands unbounded memory.
const MAX_PAGE_DIMENSION_PX: u32 = 100_000;

/// Dedicated single-thread pool for PDFium operations.
static PDF_POOL: LazyLock<rayon::ThreadPool> = LazyLock::new(|| {
    rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .thread_name(|_| "pdfium".into())
        .build()
        .expect("failed to create PDFium thread pool")
});

thread_local! {
    static RENDERER: RefCell<Option<Binding>> = const { RefCell::new(None) };
}

/// Render every page of `pdf_bytes` to a [`RenderedPage`] at `scale`, on the
/// dedicated PDFium thread where the binding is valid.
pub(super) fn render(pdf_bytes: Vec<u8>, scale: f32) -> Result<Vec<RenderedPage>> {
    PDF_POOL.install(move || {
        RENDERER.with_borrow_mut(|slot| {
            if slot.is_none() {
                *slot = Some(Binding::new()?);
            }
            slot.as_ref().unwrap().render_all(&pdf_bytes, scale)
        })
    })
}

/// Observe every page of `pdf_bytes` at `scale`: render it to RGB8 pixels and
/// extract its text-layer glyphs in rendered-pixel space, on the dedicated
/// PDFium thread.
pub(crate) fn observe(pdf_bytes: Vec<u8>, scale: f32) -> Result<Vec<PageObservation>> {
    PDF_POOL.install(move || {
        RENDERER.with_borrow_mut(|slot| {
            if slot.is_none() {
                *slot = Some(Binding::new()?);
            }
            slot.as_ref().unwrap().observe_all(&pdf_bytes, scale)
        })
    })
}

/// A PDFium binding, lazily initialised on the dedicated render thread.
struct Binding {
    pdfium: Pdfium,
}

impl Binding {
    fn new() -> Result<Self> {
        let bindings = Pdfium::bind_to_system_library()
            .or_else(|_| Pdfium::bind_to_library("libpdfium"))
            .map_err(|e| Error::invalid_document(format!("failed to load PDFium library: {e}")))?;
        Ok(Self {
            pdfium: Pdfium::new(bindings),
        })
    }

    fn render_all(&self, pdf_bytes: &[u8], scale: f32) -> Result<Vec<RenderedPage>> {
        let document = self
            .pdfium
            .load_pdf_from_byte_slice(pdf_bytes, None)
            .map_err(|e| Error::invalid_document(format!("failed to load PDF: {e}")))?;
        let config = PdfRenderConfig::new().scale_page_by_factor(scale);

        let mut pages = Vec::new();
        for page in document.pages().iter() {
            let bitmap = page
                .render_with_config(&config)
                .map_err(|e| Error::invalid_document(format!("failed to render PDF page: {e}")))?;
            let image = bitmap.as_image().map_err(|e| {
                Error::invalid_document(format!("failed to convert PDF page bitmap: {e}"))
            })?;
            let (width, height) = image.dimensions();
            let mut png = std::io::Cursor::new(Vec::new());
            image
                .write_to(&mut png, ImageFormat::Png)
                .map_err(|e| Error::invalid_document(format!("failed to encode page PNG: {e}")))?;
            pages.push(RenderedPage {
                png: png.into_inner(),
                width,
                height,
            });
        }
        Ok(pages)
    }

    /// Render each page to RGB8 pixels and extract its text-layer glyphs in
    /// rendered-pixel space (top-left origin), converting PDFium's point boxes
    /// (bottom-left origin) with the page's point→pixel scale and a Y-flip.
    fn observe_all(&self, pdf_bytes: &[u8], scale: f32) -> Result<Vec<PageObservation>> {
        let document = self
            .pdfium
            .load_pdf_from_byte_slice(pdf_bytes, None)
            .map_err(|e| Error::invalid_document(format!("failed to load PDF: {e}")))?;
        let config = PdfRenderConfig::new().scale_page_by_factor(scale);

        let page_count = document.pages().len() as usize;
        if page_count > MAX_PAGES {
            return Err(Error::invalid_document(format!(
                "document has {page_count} pages, over the {MAX_PAGES}-page render limit"
            )));
        }

        let mut observations = Vec::new();
        for (index, page) in document.pages().iter().enumerate() {
            let bitmap = page
                .render_with_config(&config)
                .map_err(|e| Error::invalid_document(format!("failed to render PDF page: {e}")))?;
            let image = bitmap
                .as_image()
                .map_err(|e| {
                    Error::invalid_document(format!("failed to convert PDF page bitmap: {e}"))
                })?
                .into_rgb8();
            let (width, height) = image.dimensions();
            if width > MAX_PAGE_DIMENSION_PX || height > MAX_PAGE_DIMENSION_PX {
                return Err(Error::invalid_document(format!(
                    "page {} renders to {width}x{height} px, over the \
                     {MAX_PAGE_DIMENSION_PX}px per-side render limit",
                    index + 1
                )));
            }
            let pixels = image.into_raw();

            // Point→pixel scale per axis, from the actual rendered dimensions
            // against the page's point size (robust to any rounding PDFium does).
            let page_w = page.width().value.max(f32::MIN_POSITIVE);
            let page_h = page.height().value.max(f32::MIN_POSITIVE);
            let scale_x = width as f32 / page_w;
            let scale_y = height as f32 / page_h;

            let mut text = String::new();
            let mut glyphs = Vec::new();
            // Running UTF-16 offset, so `start`/`end` share the page text's
            // coordinate system without re-counting the whole string per char.
            let mut utf16_offset: u32 = 0;
            if let Ok(page_text) = page.text() {
                for ch in page_text.chars().iter() {
                    let Some(c) = ch.unicode_char() else {
                        continue; // no glyph text (e.g. a control char)
                    };
                    let start = utf16_offset;
                    text.push(c);
                    utf16_offset += c.len_utf16() as u32;
                    let end = utf16_offset;
                    if let Ok(b) = ch.loose_bounds() {
                        glyphs.push(Glyph {
                            start,
                            end,
                            rect: PixelRect::from_points(
                                b.left().value,
                                b.bottom().value,
                                b.right().value,
                                b.top().value,
                                page_h,
                                scale_x,
                                scale_y,
                            ),
                            source: GlyphSource::Text,
                        });
                    }
                }
            }

            observations.push(PageObservation {
                page: (index as u32) + 1,
                width,
                height,
                text,
                glyphs,
                pixels,
            });
        }
        Ok(observations)
    }
}
