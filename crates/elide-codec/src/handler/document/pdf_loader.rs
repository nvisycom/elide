//! PDF loader: extract page text via [`elide_pdf`] and produce the handler.
//!
//! With the `pdf-render` feature the loader also carries an [`OcrMode`]: under
//! [`OcrMode::Force`] it rasterises every page to an image up front, so a
//! scanned PDF can feed the image/OCR pipeline even with no text layer.
//!
//! [`OcrMode`]: super::OcrMode
//! [`OcrMode::Force`]: super::OcrMode::Force

use elide_core::Result;
use elide_core::modality::text::Text;

#[cfg(feature = "pdf-render")]
use super::OcrMode;
use super::pdf_handler::{PdfHandler, PdfPage, pdf_error};
use crate::Loader;
use crate::content::ContentData;

/// Loader producing the [`PdfHandler`]: born-digital text extraction, plus the
/// optional page-rendering path (feature `pdf-render`).
#[derive(Debug, Default)]
pub(crate) struct PdfLoader {
    /// How to treat OCR: whether to render pages to images on decode. Only
    /// meaningful with the `pdf-render` feature, which can actually render.
    #[cfg(feature = "pdf-render")]
    ocr: OcrMode,
}

impl PdfLoader {
    /// A loader on the born-digital text path (no page rendering).
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
        let document = content.to_bytes();

        // Force-OCR: rasterise the pages up front for the image/OCR pipeline.
        #[cfg(feature = "pdf-render")]
        if let Some(dpi) = self.ocr.render_dpi() {
            let pages = render_pages(&document, dpi.scale_factor())?;
            return Ok(PdfHandler::rendered(document, pages));
        }

        // Text path: extract each page's born-digital text into stream pages.
        let extraction = elide_pdf::Pdf::open(&document)
            .map_err(pdf_error)?
            .extract();
        let mut pages = Vec::with_capacity(extraction.blocks.len());
        let mut offset = 0usize;
        for block in extraction.blocks {
            let text = block.text.to_string();
            let len = text.len();
            pages.push(PdfPage {
                number: block.page,
                text,
                start: offset,
            });
            offset += len;
        }
        Ok(PdfHandler::text(document, pages))
    }
}

/// Render every page of `document` to an [`ImageData`] at `scale`, via
/// [`elide_pdf`]'s `render` feature, converting each PNG-encoded page.
#[cfg(feature = "pdf-render")]
fn render_pages(
    document: &[u8],
    scale: f32,
) -> Result<Vec<elide_core::modality::image::ImageData>> {
    use elide_core::modality::image::ImageData;
    use elide_core::primitive::Dimensions;
    use elide_pdf::render::PdfRender;

    let rendered = elide_pdf::Pdf::open(document)
        .map_err(pdf_error)?
        .render(scale)
        .map_err(pdf_error)?;
    Ok(rendered
        .into_iter()
        .map(|page| {
            ImageData::new(
                bytes::Bytes::from(page.png),
                Dimensions::new(page.width, page.height),
            )
        })
        .collect())
}
