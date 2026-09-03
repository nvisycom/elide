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

pub use self::emit::Certificate;
pub use self::geometry::{Glyph, GlyphSource, PageObservation, PixelRect};
pub use self::raster::Detection;
#[cfg(feature = "test-utils")]
pub use self::raster::verify_raster_coverage;
use crate::Pdf;
use crate::error::Result;

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

impl Pdf {
    /// Render every page of the document to a PNG image at `scale` (1.0 is the
    /// page's natural size; e.g. 2.0 doubles resolution).
    ///
    /// For a scanned or image-only PDF whose text cannot be extracted, this
    /// produces page images an OCR engine can read. Requires the PDFium shared
    /// library at runtime.
    ///
    /// The pristine bytes the document was opened from are rendered, so a
    /// redaction is not reflected; to rasterise a redacted document, re-open its
    /// output bytes and render that.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidDocument`](crate::ErrorKind::InvalidDocument) if
    /// PDFium cannot load or render the document (or the native library is
    /// unavailable).
    #[cfg_attr(docsrs, doc(cfg(feature = "render")))]
    pub fn render(&self, scale: f32) -> Result<Vec<RenderedPage>> {
        // Render the pristine bytes this document was opened from, not a lopdf
        // re-serialisation, which can degrade the scanned or malformed PDFs OCR
        // most needs, on the dedicated PDFium thread.
        pdfium::render(self.source_bytes().to_vec(), scale)
    }

    /// Observe every page at `scale`: render it to RGB8 pixels and extract its
    /// text-layer glyphs in rendered-pixel space.
    ///
    /// This is the input to raster redaction: each [`PageObservation`] carries
    /// the pixels to overwrite, the page text detection runs over, and the
    /// glyph boxes that map a detected UTF-16 span back to pixels. A page with
    /// no text layer yields an observation with pixels but no glyphs, a caller
    /// supplies OCR glyphs for those.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidDocument`](crate::ErrorKind::InvalidDocument) if
    /// PDFium cannot load or render the document (or the native library is
    /// unavailable).
    #[cfg_attr(docsrs, doc(cfg(feature = "render")))]
    pub fn observe(&self, scale: f32) -> Result<Vec<PageObservation>> {
        pdfium::observe(self.source_bytes().to_vec(), scale)
    }
}
