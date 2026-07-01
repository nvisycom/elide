//! PDF loader: accepts the bytes and produces the handler.
//!
//! Text parsing is not implemented yet (see [`PdfHandler`]). With the
//! `pdf-render` feature the loader also carries an [`OcrMode`]: under
//! [`OcrMode::Force`] it rasterises every page to an image up front, so a
//! scanned PDF can feed the image/OCR pipeline even with no text layer.
//!
//! [`OcrMode`]: super::OcrMode
//! [`OcrMode::Force`]: super::OcrMode::Force

use elide_core::Result;
use elide_core::modality::text::Text;

#[cfg(feature = "pdf-render")]
use super::OcrMode;
use super::pdf_handler::PdfHandler;
#[cfg(feature = "pdf-render")]
use super::pdf_render::render_pages;
use crate::Loader;
use crate::content::ContentData;

/// Loader producing the [`PdfHandler`]. Text validation will arrive with
/// the real object parser; today its only behaviour is the optional
/// page-rendering path (feature `pdf-render`).
#[derive(Debug, Default)]
pub(crate) struct PdfLoader {
    /// How to treat OCR: whether to render pages to images on decode. Only
    /// meaningful with the `pdf-render` feature, which can actually render.
    #[cfg(feature = "pdf-render")]
    ocr: OcrMode,
}

impl PdfLoader {
    /// A loader on the plain text path (no page rendering).
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A loader that renders pages for OCR per the given [`OcrMode`].
    ///
    /// [`OcrMode`]: super::OcrMode
    #[cfg(feature = "pdf-render")]
    pub(crate) fn with_ocr(ocr: OcrMode) -> Self {
        Self { ocr }
    }
}

#[async_trait::async_trait]
impl Loader<Text> for PdfLoader {
    type Handler = PdfHandler;

    async fn decode(&self, content: ContentData) -> Result<PdfHandler> {
        // Render the pages up front when forced; the rendered images ride
        // on the handler for the image/OCR pipeline to pick up.
        #[cfg(feature = "pdf-render")]
        if let Some(dpi) = self.ocr.render_dpi() {
            let pages = render_pages(content.as_bytes(), dpi)?;
            return Ok(PdfHandler::with_pages(pages));
        }

        let _ = content;
        Ok(PdfHandler::new())
    }
}
