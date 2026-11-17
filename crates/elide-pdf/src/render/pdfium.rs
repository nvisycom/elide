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
use image::{ImageFormat, RgbImage, load_from_memory};
use pdfium_render::prelude::*;
use rayon::{ThreadPool, ThreadPoolBuilder};

use super::raster::fill;
use super::{PageReplacement, PixelRect, ScannedPage};
use crate::text::{DeviceRect, GlyphRect};

/// Most pages a document may have to be read or rendered: a page is rendered
/// to a full bitmap, so an unbounded count is a memory denial of service.
const MAX_PAGES: PdfPageIndex = 10_000;

/// Most pixels one page may render to. A render holds PDFium's four-byte
/// bitmap, a four-byte copy, and a three-byte RGB copy at once, so this bounds
/// one page near 700 MiB; an A2 page at 300 DPI (35 million pixels) fits.
const MAX_PAGE_PIXELS: f32 = 67_108_864.0;

/// Most pixels one call may render across all its pages: about 2,000 Letter
/// pages at 300 DPI. Each page's output is kept until the call returns, so a
/// document of many huge blank pages would otherwise run without bound.
const MAX_DOCUMENT_PIXELS: f32 = 17_179_869_184.0;

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

/// Every page's text and glyph boxes, in page order. A page whose text layer
/// PDFium cannot read has no text, so it is treated as scanned.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the
/// document cannot be loaded, or a glyph's box cannot be read: a character
/// with no box could be detected but never painted over.
pub(crate) fn observe(pdf_bytes: &[u8]) -> Result<Vec<(String, Vec<GlyphRect>)>> {
    with_document(pdf_bytes, |document| {
        let mut pages = Vec::new();
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
                    glyphs.push(GlyphRect {
                        chars: start..start + 1,
                        rect: DeviceRect {
                            min_x: f64::from(b.left().value),
                            min_y: f64::from(b.bottom().value),
                            max_x: f64::from(b.right().value),
                            max_y: f64::from(b.top().value),
                        },
                    });
                }
            }
            pages.push((text, glyphs));
        }
        Ok(pages)
    })
}

/// The 1-based pages `numbers` rendered at `scale` to PNG images.
///
/// # Errors
///
/// As [`each_page`], and
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a
/// page cannot be encoded.
pub(crate) fn render_pages(
    pdf_bytes: &[u8],
    numbers: &BTreeSet<u32>,
    scale: f32,
) -> Result<BTreeMap<u32, Bytes>> {
    let mut out = BTreeMap::new();
    each_page(
        pdf_bytes,
        numbers,
        scale,
        &BTreeMap::new(),
        |number, image| {
            out.insert(number, png(number, &image)?);
            Ok(())
        },
    )?;
    Ok(out)
}

/// The 1-based pages `numbers` rendered at `scale`, each with the glyph boxes
/// `boxes` names for it painted over, as the image that replaces the page.
///
/// # Errors
///
/// As [`each_page`], and as [`PageReplacement::pixels`].
pub(crate) fn flatten(
    pdf_bytes: &[u8],
    numbers: &BTreeSet<u32>,
    scale: f32,
    boxes: &BTreeMap<u32, Vec<DeviceRect>>,
) -> Result<Vec<PageReplacement>> {
    let mut out = Vec::new();
    each_page(pdf_bytes, numbers, scale, boxes, |number, image| {
        out.push(PageReplacement::pixels(number, image)?);
        Ok(())
    })?;
    Ok(out)
}

/// Each of the `scanned` pages rendered at `scale` with its redaction laid
/// over: every pixel the redacted image changed from the page's original
/// render, so what the document now draws (its redacted embedded images)
/// shows everywhere else.
///
/// # Errors
///
/// As [`each_page`];
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if an
/// image cannot be decoded, and
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if its size is
/// not the page's.
pub(crate) fn composite(
    pdf_bytes: &[u8],
    scanned: &[ScannedPage],
    scale: f32,
) -> Result<Vec<PageReplacement>> {
    let numbers: BTreeSet<u32> = scanned.iter().map(|page| page.number).collect();
    let mut out = Vec::new();
    each_page(
        pdf_bytes,
        &numbers,
        scale,
        &BTreeMap::new(),
        |number, mut image| {
            let Some(page) = scanned.iter().find(|page| page.number == number) else {
                unreachable!("only the scanned pages are rendered");
            };
            let original = decode(number, &page.original)?;
            let redacted = decode(number, &page.redacted)?;
            if original.dimensions() != image.dimensions()
                || redacted.dimensions() != image.dimensions()
            {
                return Err(Error::redaction(format!(
                    "page {number}'s redacted image is not the size of the page"
                )));
            }
            for ((pixel, before), after) in image
                .pixels_mut()
                .zip(original.pixels())
                .zip(redacted.pixels())
            {
                if before != after {
                    *pixel = *after;
                }
            }
            out.push(PageReplacement::pixels(number, image)?);
            Ok(())
        },
    )?;
    Ok(out)
}

/// Render each of the 1-based pages `numbers` at `scale`, paint over the boxes
/// `boxes` names for it, and hand it to `visit`, on the PDFium thread.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the
/// document cannot be loaded, a page cannot be rendered, or a number names no
/// page; [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) for
/// a page, or pages together, too large to render;
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) for a box that
/// cannot be placed; or as `visit`.
fn each_page(
    pdf_bytes: &[u8],
    numbers: &BTreeSet<u32>,
    scale: f32,
    boxes: &BTreeMap<u32, Vec<DeviceRect>>,
    mut visit: impl FnMut(u32, RgbImage) -> Result<()> + Send,
) -> Result<()> {
    with_document(pdf_bytes, |document| {
        let config = PdfRenderConfig::new().scale_page_by_factor(scale);
        let mut rendered = 0usize;
        let mut total = 0.0f32;
        for (number, page) in (1u32..).zip(document.pages().iter()) {
            if !numbers.contains(&number) {
                continue;
            }
            let pixels = page.width().value * scale * page.height().value * scale;
            total += pixels;
            if pixels.is_nan() || pixels > MAX_PAGE_PIXELS {
                return Err(Error::resource_limit(format!(
                    "page {number} would render to {pixels} pixels, over the \
                     {MAX_PAGE_PIXELS}-pixel limit"
                )));
            }
            if total > MAX_DOCUMENT_PIXELS {
                return Err(Error::resource_limit(format!(
                    "the pages would render to more than {MAX_DOCUMENT_PIXELS} pixels"
                )));
            }
            let mut image = page
                .render_with_config(&config)
                .and_then(|bitmap| bitmap.as_image())
                .map(|image| image.into_rgb8())
                .map_err(|e| Error::malformed(format!("failed to render page {number}: {e}")))?;
            for glyph in boxes.get(&number).into_iter().flatten() {
                let mut corners = [(0, 0); 4];
                for (corner, (x, y)) in corners.iter_mut().zip(glyph.corners()) {
                    let (x, y) = (PdfPoints::new(x as f32), PdfPoints::new(y as f32));
                    *corner = page.points_to_pixels(x, y, &config).map_err(|e| {
                        Error::redaction(format!("page {number} glyph box cannot be placed: {e}"))
                    })?;
                }
                fill(&mut image, PixelRect::covering(corners));
            }
            visit(number, image)?;
            rendered += 1;
        }
        if rendered == numbers.len() {
            return Ok(());
        }
        Err(Error::malformed(format!(
            "asked to render {} pages, but the document has only {rendered} of them",
            numbers.len()
        )))
    })
}

/// `image` encoded as a PNG.
fn png(number: u32, image: &RgbImage) -> Result<Bytes> {
    let mut png = Cursor::new(Vec::new());
    image
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|e| Error::malformed(format!("failed to encode page {number}: {e}")))?;
    Ok(Bytes::from(png.into_inner()))
}

/// The pixels of page `number`'s encoded image `bytes`.
fn decode(number: u32, bytes: &[u8]) -> Result<RgbImage> {
    load_from_memory(bytes)
        .map(|image| image.into_rgb8())
        .map_err(|e| Error::malformed(format!("page {number}'s image cannot be decoded: {e}")))
}
