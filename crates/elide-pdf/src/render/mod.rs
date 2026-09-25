//! PDF page rendering via PDFium, and raster redaction built on it, behind the
//! `render` feature.
//!
//! Rendering rasterises whole pages to images (for OCR of scanned or image-only
//! PDFs, and for the raster redaction that flattens a page to a sanitised
//! image). The glyph geometry here, in rendered-pixel space, is the bridge
//! between a detected text span and the pixels the redaction overwrites. It all
//! requires the PDFium shared library at runtime (see
//! `scripts/install-pdfium.sh`); the module is behind the `render` feature so
//! the default build needs no native library.
//!
//! The geometry types carry no elide dependency, so the crate stays standalone.

mod emit;
mod geometry;
mod pdfium;
mod raster;

use std::collections::{BTreeMap, BTreeSet};

use elide_core::Result;

pub use self::emit::Certificate;
pub use self::geometry::{Glyph, GlyphSource, PageObservation, PixelRect};
pub(crate) use self::raster::redact_raster;
#[cfg(feature = "test-utils")]
pub use self::raster::verify_raster_coverage;
use crate::document::Store;

/// A page rendered to a PNG image, with its pixel dimensions.
///
/// Deliberately free of any elide type so the crate stays self-contained; a
/// caller wraps `png` into its own image representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedPage {
    /// PNG-encoded page image.
    pub png: Vec<u8>,
    /// Rendered width in pixels.
    pub width: u32,
    /// Rendered height in pixels.
    pub height: u32,
}

/// Render only the 1-based pages in `numbers` at `scale`, returning each keyed
/// by its page number.
///
/// Pages not named are not rasterised, so a pass that needs only a few pages
/// (e.g. the scanned pages of a mostly born-digital document) does not pay to
/// render the whole document.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](crate::ErrorKind::MalformedInput) if PDFium
/// cannot load or render the document (or the native library is unavailable).
#[cfg(feature = "image")]
pub(crate) fn render_pages(
    store: &Store,
    numbers: BTreeSet<u32>,
    scale: f32,
) -> Result<BTreeMap<u32, RenderedPage>> {
    pdfium::render_pages(store.source_bytes().to_vec(), numbers, scale)
}

/// Observe every page at `scale`: render it to RGB8 pixels and extract its
/// text-layer glyphs in rendered-pixel space.
///
/// This is the input to raster redaction: each [`PageObservation`] carries the
/// pixels to overwrite, the page text detection runs over, and the glyph boxes
/// that map a detected character span back to pixels. A page with no text layer
/// yields an observation with pixels but no glyphs, a caller supplies OCR glyphs
/// for those.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](crate::ErrorKind::MalformedInput) if PDFium
/// cannot load or render the document (or the native library is unavailable).
pub(crate) fn observe(store: &Store, scale: f32) -> Result<Vec<PageObservation>> {
    pdfium::observe(store.source_bytes().to_vec(), scale)
}
