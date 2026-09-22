//! The PDFium binding and its dedicated render thread.
//!
//! PDFium is not thread-safe and an open document borrows the binding, so all
//! rendering is serialised on a dedicated single-thread pool; the binding is
//! created once on first use via a `thread_local!` and reused.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use elide_core::{Error, ErrorKind, Result};
use image::{GenericImageView, ImageFormat};
use pdfium_render::prelude::*;

use super::{Glyph, GlyphSource, PageObservation, PixelRect, RenderedPage};

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

/// Render only the 1-based pages in `numbers`, returning each keyed by its page
/// number, on the dedicated PDFium thread. Pages not in `numbers` are not
/// rendered at all (so a scanned-page pass over a mostly-text document does not
/// pay to rasterise every page).
pub(crate) fn render_pages(
    pdf_bytes: Vec<u8>,
    numbers: BTreeSet<u32>,
    scale: f32,
) -> Result<BTreeMap<u32, RenderedPage>> {
    PDF_POOL.install(move || {
        RENDERER.with_borrow_mut(|slot| {
            if slot.is_none() {
                *slot = Some(Binding::new()?);
            }
            slot.as_ref()
                .unwrap()
                .render_pages(&pdf_bytes, &numbers, scale)
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
            .map_err(|e| {
                Error::new(
                    ErrorKind::MalformedInput,
                    format!("failed to load PDFium library: {e}"),
                )
            })?;
        Ok(Self {
            pdfium: Pdfium::new(bindings),
        })
    }

    fn render_all(&self, pdf_bytes: &[u8], scale: f32) -> Result<Vec<RenderedPage>> {
        let document = self
            .pdfium
            .load_pdf_from_byte_slice(pdf_bytes, None)
            .map_err(|e| {
                Error::new(
                    ErrorKind::MalformedInput,
                    format!("failed to load PDF: {e}"),
                )
            })?;
        let config = PdfRenderConfig::new().scale_page_by_factor(scale);

        let mut pages = Vec::new();
        for page in document.pages().iter() {
            pages.push(render_page(&page, &config)?);
        }
        Ok(pages)
    }

    /// Render only the 1-based pages in `numbers`, keyed by page number.
    fn render_pages(
        &self,
        pdf_bytes: &[u8],
        numbers: &BTreeSet<u32>,
        scale: f32,
    ) -> Result<BTreeMap<u32, RenderedPage>> {
        let document = self
            .pdfium
            .load_pdf_from_byte_slice(pdf_bytes, None)
            .map_err(|e| {
                Error::new(
                    ErrorKind::MalformedInput,
                    format!("failed to load PDF: {e}"),
                )
            })?;
        let config = PdfRenderConfig::new().scale_page_by_factor(scale);

        let page_count = document.pages().len() as usize;
        if page_count > MAX_PAGES {
            return Err(Error::new(
                ErrorKind::ResourceLimit,
                format!("document has {page_count} pages, over the {MAX_PAGES}-page render limit"),
            ));
        }

        let mut out = BTreeMap::new();
        for (index, page) in document.pages().iter().enumerate() {
            // Page objects are 0-based here; the API numbers pages from 1.
            let Some(number) = u32::try_from(index).ok().and_then(|i| i.checked_add(1)) else {
                continue;
            };
            if numbers.contains(&number) {
                out.insert(number, render_page(&page, &config)?);
            }
        }
        Ok(out)
    }

    /// Render each page to RGB8 pixels and extract its text-layer glyphs in
    /// rendered-pixel space (top-left origin), converting PDFium's point boxes
    /// (bottom-left origin) with the page's point→pixel scale and a Y-flip.
    fn observe_all(&self, pdf_bytes: &[u8], scale: f32) -> Result<Vec<PageObservation>> {
        let document = self
            .pdfium
            .load_pdf_from_byte_slice(pdf_bytes, None)
            .map_err(|e| {
                Error::new(
                    ErrorKind::MalformedInput,
                    format!("failed to load PDF: {e}"),
                )
            })?;
        let config = PdfRenderConfig::new().scale_page_by_factor(scale);

        let page_count = document.pages().len() as usize;
        if page_count > MAX_PAGES {
            return Err(Error::new(
                ErrorKind::ResourceLimit,
                format!("document has {page_count} pages, over the {MAX_PAGES}-page render limit"),
            ));
        }

        let mut observations = Vec::new();
        for (index, page) in document.pages().iter().enumerate() {
            let bitmap = page.render_with_config(&config).map_err(|e| {
                Error::new(
                    ErrorKind::MalformedInput,
                    format!("failed to render PDF page: {e}"),
                )
            })?;
            let image = bitmap
                .as_image()
                .map_err(|e| {
                    Error::new(
                        ErrorKind::MalformedInput,
                        format!("failed to convert PDF page bitmap: {e}"),
                    )
                })?
                .into_rgb8();
            let (width, height) = image.dimensions();
            if width > MAX_PAGE_DIMENSION_PX || height > MAX_PAGE_DIMENSION_PX {
                return Err(Error::new(
                    ErrorKind::ResourceLimit,
                    format!(
                        "page {} renders to {width}x{height} px, over the \
                     {MAX_PAGE_DIMENSION_PX}px per-side render limit",
                        index + 1
                    ),
                ));
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
            // Running character offset, so `start`/`end` share the page text's
            // coordinate system (character offsets, as a `Detection` carries)
            // without re-counting the whole string per char.
            let mut char_offset: usize = 0;
            if let Ok(page_text) = page.text() {
                for ch in page_text.chars().iter() {
                    let Some(c) = ch.unicode_char() else {
                        continue; // no glyph text (e.g. a control char)
                    };
                    let start = char_offset;
                    text.push(c);
                    char_offset += 1;
                    let end = char_offset;
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

/// Render one page to a PNG at `config`'s scale.
fn render_page(page: &PdfPage, config: &PdfRenderConfig) -> Result<RenderedPage> {
    let bitmap = page.render_with_config(config).map_err(|e| {
        Error::new(
            ErrorKind::MalformedInput,
            format!("failed to render PDF page: {e}"),
        )
    })?;
    let image = bitmap.as_image().map_err(|e| {
        Error::new(
            ErrorKind::MalformedInput,
            format!("failed to convert PDF page bitmap: {e}"),
        )
    })?;
    let (width, height) = image.dimensions();
    // Reject an oversized render before encoding, so a page that rasterises to a
    // huge bitmap cannot drive unbounded PNG-encoding memory (the same bound
    // `observe_all` applies to the glyph-observation path).
    if width > MAX_PAGE_DIMENSION_PX || height > MAX_PAGE_DIMENSION_PX {
        return Err(Error::new(
            ErrorKind::ResourceLimit,
            format!(
                "page renders to {width}x{height} px, over the \
                 {MAX_PAGE_DIMENSION_PX}px per-side render limit"
            ),
        ));
    }
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png).map_err(|e| {
        Error::new(
            ErrorKind::MalformedInput,
            format!("failed to encode page PNG: {e}"),
        )
    })?;
    Ok(RenderedPage {
        png: png.into_inner(),
        width,
        height,
    })
}
