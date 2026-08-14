//! PDF-to-image rendering via PDFium, behind the `render` feature.
//!
//! Rendering rasterises whole pages to images (for OCR of scanned or
//! image-only PDFs). It requires the PDFium shared library at runtime (see
//! `scripts/install-pdfium.sh`); the whole module is behind the `render`
//! feature so the default build needs no native library.

mod pdfium;

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

/// Rasterising a PDF's pages to images.
///
/// Implemented for [`Pdf`] under the `render` feature. It is a trait so the
/// native rendering capability is a distinct, opt-in extension of the core
/// text-extraction API rather than an inherent method.
#[cfg_attr(docsrs, doc(cfg(feature = "render")))]
pub trait PdfRender {
    /// Render every page of the document to a PNG image at `scale` (1.0 is the
    /// page's natural size; e.g. 2.0 doubles resolution).
    ///
    /// For a scanned or image-only PDF whose text cannot be extracted, this
    /// produces page images an OCR engine can read. Requires the PDFium shared
    /// library at runtime.
    ///
    /// The pristine bytes the document was opened from are rendered, so a
    /// rewrite is not reflected; to rasterise a rewritten document, re-open its
    /// output bytes and render that.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidDocument`](crate::ErrorKind::InvalidDocument) if
    /// PDFium cannot load or render the document (or the native library is
    /// unavailable).
    fn render(&self, scale: f32) -> Result<Vec<RenderedPage>>;
}

impl PdfRender for Pdf {
    fn render(&self, scale: f32) -> Result<Vec<RenderedPage>> {
        // Render the pristine bytes this document was opened from — not a lopdf
        // re-serialisation, which can degrade the scanned or malformed PDFs OCR
        // most needs — on the dedicated PDFium thread.
        pdfium::render(self.source().to_vec(), scale)
    }
}
