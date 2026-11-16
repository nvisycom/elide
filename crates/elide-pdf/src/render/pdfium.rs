//! The PDFium binding and its dedicated render thread.
//!
//! PDFium is not thread-safe and an open document borrows the binding, so all
//! rendering is serialised on a dedicated single-thread pool; the binding is
//! created once on first use via a `thread_local!` and reused.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::sync::LazyLock;

use bytes::Bytes;
use elide_core::{Error, Result};
use image::{ImageFormat, RgbImage};
use pdfium_render::prelude::*;
use rayon::{ThreadPool, ThreadPoolBuilder};

use super::raster::fill;
use super::{Glyph, GlyphBox, PageObservation, PageReplacement, PixelRect};

/// Most pages a document may have to be read or rendered: a page is rendered
/// to a full bitmap, so an unbounded count is a memory denial of service.
const MAX_PAGES: PdfPageIndex = 10_000;

/// Most pixels one page may render to: PDFium allocates four bytes a pixel,
/// so this bounds a page's bitmap at 1 GiB, past an A0 page at 300 DPI.
const MAX_PAGE_PIXELS: f32 = 268_435_456.0;

/// Dedicated single-thread pool for PDFium operations.
static PDF_POOL: LazyLock<ThreadPool> = LazyLock::new(|| {
    ThreadPoolBuilder::new()
        .num_threads(1)
        .thread_name(|_| "pdfium".into())
        .build()
        .expect("failed to create PDFium thread pool")
});

thread_local! {
    static RENDERER: RefCell<Option<Pdfium>> = const { RefCell::new(None) };
}

/// Run `task` with the loaded `pdf_bytes` on the PDFium thread.
fn with_document<T: Send>(
    pdf_bytes: &[u8],
    task: impl FnOnce(&PdfDocument<'_>) -> Result<T> + Send,
) -> Result<T> {
    PDF_POOL.install(|| {
        RENDERER.with_borrow_mut(|slot| {
            if slot.is_none() {
                *slot = Some(bind()?);
            }
            let Some(pdfium) = slot.as_ref() else {
                unreachable!("the binding was just created");
            };
            let document = pdfium
                .load_pdf_from_byte_slice(pdf_bytes, None)
                .map_err(|e| Error::malformed(format!("failed to load PDF: {e}")))?;
            let count = document.pages().len();
            if count > MAX_PAGES {
                return Err(Error::resource_limit(format!(
                    "document has {count} pages, over the {MAX_PAGES}-page render limit"
                )));
            }
            task(&document)
        })
    })
}

/// Bind the PDFium library: the system one, else one beside the binary.
fn bind() -> Result<Pdfium> {
    let bindings = Pdfium::bind_to_system_library().or_else(|system| {
        Pdfium::bind_to_library("libpdfium").map_err(|local| {
            Error::malformed(format!(
                "failed to load PDFium library: {system}; fallback: {local}"
            ))
        })
    })?;
    Ok(Pdfium::new(bindings))
}

/// Every page's text and glyph boxes. A page whose text layer PDFium cannot
/// read has no text, so it is treated as scanned.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the
/// document cannot be loaded, or a glyph's box cannot be read: a character
/// with no box could be detected but never painted over.
pub(crate) fn observe(pdf_bytes: &[u8]) -> Result<Vec<PageObservation>> {
    with_document(pdf_bytes, |document| {
        let mut observations = Vec::new();
        for (number, page) in (1u32..).zip(document.pages().iter()) {
            let mut text = String::new();
            let mut glyphs = Vec::new();
            if let Ok(page_text) = page.text() {
                for (start, (c, ch)) in page_text
                    .chars()
                    .iter()
                    .filter_map(|ch| Some((ch.unicode_char()?, ch)))
                    .enumerate()
                {
                    text.push(c);
                    let b = ch.loose_bounds().map_err(|e| {
                        Error::malformed(format!("page {number} glyph {start} has no box: {e}"))
                    })?;
                    glyphs.push(Glyph {
                        start,
                        end: start + 1,
                        bounds: GlyphBox {
                            left: b.left().value,
                            bottom: b.bottom().value,
                            right: b.right().value,
                            top: b.top().value,
                        },
                    });
                }
            }
            observations.push(PageObservation {
                page: number,
                text,
                glyphs,
            });
        }
        Ok(observations)
    })
}

/// The 1-based pages `numbers` rendered at `scale` to PNG images.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the
/// document cannot be loaded, a page cannot be rendered, or a number names no
/// page; [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) for
/// a page that renders too large.
pub(crate) fn render_pages(
    pdf_bytes: &[u8],
    numbers: &BTreeSet<u32>,
    scale: f32,
) -> Result<BTreeMap<u32, Bytes>> {
    with_document(pdf_bytes, |document| {
        let config = PdfRenderConfig::new().scale_page_by_factor(scale);
        let mut out = BTreeMap::new();
        for (number, page) in (1u32..).zip(document.pages().iter()) {
            if !numbers.contains(&number) {
                continue;
            }
            let image = render(&page, number, scale, &config)?;
            let mut png = Cursor::new(Vec::new());
            image
                .write_to(&mut png, ImageFormat::Png)
                .map_err(|e| Error::malformed(format!("failed to encode page {number}: {e}")))?;
            out.insert(number, Bytes::from(png.into_inner()));
        }
        every_page(numbers, out.len())?;
        Ok(out)
    })
}

/// The 1-based pages `numbers` rendered at `scale`, each with the glyph boxes
/// `fill` names for it painted over, as the image that replaces the page.
///
/// # Errors
///
/// As [`render_pages`], and as [`PageReplacement::pixels`].
pub(crate) fn flatten(
    pdf_bytes: &[u8],
    numbers: &BTreeSet<u32>,
    scale: f32,
    boxes: &BTreeMap<u32, Vec<GlyphBox>>,
) -> Result<Vec<PageReplacement>> {
    with_document(pdf_bytes, |document| {
        let config = PdfRenderConfig::new().scale_page_by_factor(scale);
        let mut out = Vec::new();
        for (number, page) in (1u32..).zip(document.pages().iter()) {
            if !numbers.contains(&number) {
                continue;
            }
            let mut image = render(&page, number, scale, &config)?;
            for glyph in boxes.get(&number).into_iter().flatten() {
                let mut pixels = [(0, 0); 4];
                for (pixel, (x, y)) in pixels.iter_mut().zip(glyph.corners()) {
                    *pixel = page
                        .points_to_pixels(PdfPoints::new(x), PdfPoints::new(y), &config)
                        .map_err(|e| {
                            Error::redaction(format!(
                                "page {number} glyph box cannot be placed: {e}"
                            ))
                        })?;
                }
                fill(&mut image, PixelRect::covering(pixels));
            }
            out.push(PageReplacement::pixels(number, image)?);
        }
        every_page(numbers, out.len())?;
        Ok(out)
    })
}

/// Render `page` (numbered `number`) at `config` to RGB pixels.
///
/// The page's size is checked before PDFium allocates its bitmap, so a page
/// claiming an enormous box is refused rather than rendered.
fn render(
    page: &PdfPage<'_>,
    number: u32,
    scale: f32,
    config: &PdfRenderConfig,
) -> Result<RgbImage> {
    let pixels = page.width().value * scale * page.height().value * scale;
    if pixels.is_nan() || pixels > MAX_PAGE_PIXELS {
        return Err(Error::resource_limit(format!(
            "page {number} would render to {pixels} pixels, over the \
             {MAX_PAGE_PIXELS}-pixel limit"
        )));
    }
    let bitmap = page
        .render_with_config(config)
        .map_err(|e| Error::malformed(format!("failed to render page {number}: {e}")))?;
    bitmap
        .as_image()
        .map(|image| image.into_rgb8())
        .map_err(|e| Error::malformed(format!("failed to convert page {number}: {e}")))
}

/// Refuse a render that produced fewer pages than were asked for.
fn every_page(numbers: &BTreeSet<u32>, rendered: usize) -> Result<()> {
    if rendered == numbers.len() {
        return Ok(());
    }
    Err(Error::malformed(format!(
        "asked to render {} pages, but the document has only {rendered} of them",
        numbers.len()
    )))
}
