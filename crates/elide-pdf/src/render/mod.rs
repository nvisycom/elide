//! PDF page rendering via PDFium, and raster redaction built on it, behind the
//! `render` feature.
//!
//! A page is rendered to pixels for two things: a scanned page (no text layer)
//! is read by the image pipeline for OCR, and the regions it redacts are painted
//! over the page as the document then draws it (its redacted images showing),
//! and under [`RasterMode::Always`](crate::primitive::RasterMode::Always) every
//! page is rendered with its detected glyphs painted over. Either way the page
//! is then written back as that one image ([`reflatten`](fn@reflatten)). It all
//! requires the PDFium shared library at runtime
//! (see `scripts/install-pdfium.sh`).

mod geometry;
mod pdfium;
mod raster;
mod reflatten;

use std::collections::{BTreeMap, BTreeSet};

use bytes::Bytes;
use elide_core::{Error, Result};

pub(crate) use self::geometry::{PageObservation, PixelRect};
pub(crate) use self::raster::glyph_boxes;
pub(crate) use self::reflatten::{PageReplacement, ScannedPage, reflatten};
use crate::document::Store;
use crate::text::{DeviceRect, PageFrame, page_frame};

/// The 1-based pages `numbers` of the document in `store`, rendered at `scale`
/// to PNG images keyed by page number.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
/// PDFium cannot load or render the document (or the native library is
/// unavailable), or
/// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if it has
/// too many pages or they render too large.
pub(crate) fn render_pages(
    store: &Store,
    numbers: BTreeSet<u32>,
    scale: f32,
) -> Result<BTreeMap<u32, Bytes>> {
    pdfium::render_pages(&store.bytes(), &numbers, scale)
}

/// Each of the `scanned` pages of the document in `store` rendered fresh at
/// `scale`, with the regions redacted on it painted over, as the image that
/// replaces it.
///
/// # Errors
///
/// As [`render_pages`], and
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if a fresh
/// render is not the size of the one its regions were found on.
pub(crate) fn repaint(
    store: &Store,
    scanned: &[ScannedPage],
    scale: f32,
) -> Result<Vec<PageReplacement>> {
    pdfium::repaint(&store.bytes(), scanned, scale)
}

/// Every page's text as PDFium reads it, with each glyph's box and the
/// page's frame: the input to raster redaction.
///
/// # Errors
///
/// As [`render_pages`], and
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) for a
/// glyph whose box cannot be read, or a page count PDFium and the parser
/// disagree on.
pub(crate) fn observe(store: &Store) -> Result<Vec<PageObservation>> {
    let doc = store.doc();
    let pages = doc.get_pages();
    let observed = pdfium::observe(&store.bytes())?;
    if observed.len() != pages.len() {
        return Err(Error::malformed(format!(
            "PDFium reads {} pages where the document has {}",
            observed.len(),
            pages.len()
        )));
    }
    Ok(pages
        .into_iter()
        .zip(observed)
        .map(|((page, id), (text, glyphs))| PageObservation {
            page,
            text,
            glyphs,
            frame: page_frame(doc, id).unwrap_or(PageFrame::LETTER),
        })
        .collect())
}

/// The 1-based pages `numbers` rendered at `scale` with the glyph boxes `fill`
/// names painted over, each as the image that replaces its page.
///
/// # Errors
///
/// As [`render_pages`], and
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) for a glyph box
/// that cannot be placed on its rendered page.
pub(crate) fn flatten(
    store: &Store,
    numbers: BTreeSet<u32>,
    scale: f32,
    fill: &BTreeMap<u32, Vec<DeviceRect>>,
) -> Result<Vec<PageReplacement>> {
    pdfium::flatten(&store.bytes(), &numbers, scale, fill)
}
