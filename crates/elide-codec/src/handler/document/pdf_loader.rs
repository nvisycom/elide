//! PDF loader: extract page text via [`elide_pdf`] and produce the handler.
//!
//! Redaction defaults to glyph deletion (keeps a selectable text layer). With
//! the `pdf-render` feature the loader carries a [`RasterMode`]; under
//! [`RasterMode::Always`] it observes pages for raster redaction (flatten to a
//! fresh image-only PDF) instead.
//!
//! [`RasterMode`]: super::RasterMode
//! [`RasterMode::Always`]: super::RasterMode::Always

use elide_core::Result;
use elide_core::modality::text::Text;

#[cfg(feature = "pdf-render")]
use super::RasterMode;
use super::pdf_handler::{PdfHandler, PdfPage, pdf_error};
use crate::Loader;
use crate::content::ContentData;

/// Loader producing the [`PdfHandler`]: born-digital text extraction, plus the
/// optional page-rendering path (feature `pdf-render`).
#[derive(Debug, Default)]
pub(crate) struct PdfLoader {
    /// Whether redaction flattens pages to images (raster) instead of the
    /// default glyph deletion. Only meaningful with the `pdf-render` feature.
    #[cfg(feature = "pdf-render")]
    raster: RasterMode,
}

impl PdfLoader {
    /// A loader on the born-digital text path (no page rendering).
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A loader with an explicit [`RasterMode`] (feature `pdf-render`).
    ///
    /// [`RasterMode`]: super::RasterMode
    #[cfg(feature = "pdf-render")]
    pub(crate) fn with_raster(raster: RasterMode) -> Self {
        Self { raster }
    }
}

#[async_trait::async_trait]
impl Loader<Text> for PdfLoader {
    type Handler = PdfHandler;

    async fn decode(&self, content: ContentData) -> Result<PdfHandler> {
        let document = content.to_bytes();

        // `RasterMode::Always` (feature `pdf-render`): observe each page —
        // its text comes from the renderer alongside its glyph geometry — so
        // redaction on encode fills the detected pixels and emits a fresh
        // image-only PDF (the flatten guarantee, no selectable text).
        #[cfg(feature = "pdf-render")]
        if self.raster.render_dpi().is_some() {
            let observations = observe_pages(&document)?;
            let pages = pages_from_texts(observations.iter().map(|o| (o.page, o.text.clone())));
            return Ok(PdfHandler::raster(document, pages, observations));
        }

        // Default (`Auto`/`Never`, and the whole pure-Rust build): glyph
        // deletion. The page text comes from `page_texts` — the same walk
        // `redact_text` uses — so a detection's character span maps to the
        // glyphs it drew. On encode the glyphs are deleted and
        // annotations/metadata stripped, keeping a selectable text layer.
        let pdf = elide_pdf::Pdf::open(&document).map_err(pdf_error)?;
        let pages = pages_from_texts(pdf.page_texts().map_err(pdf_error)?);
        Ok(PdfHandler::text(document, pages))
    }
}

/// Assemble [`PdfPage`]s from `(page number, text)` pairs, assigning each its
/// start offset in the concatenated text stream.
fn pages_from_texts(texts: impl IntoIterator<Item = (u32, String)>) -> Vec<PdfPage> {
    let mut pages = Vec::new();
    let mut offset = 0usize;
    for (number, text) in texts {
        let len = text.len();
        pages.push(PdfPage {
            number,
            text,
            start: offset,
        });
        offset += len;
    }
    pages
}

/// Observe every page for raster redaction: render it to pixels and extract its
/// text-layer glyph geometry, so the page text and glyph boxes share one
/// coordinate system.
#[cfg(feature = "pdf-render")]
fn observe_pages(document: &[u8]) -> Result<Vec<elide_pdf::render::PageObservation>> {
    // A default render scale; higher scales trade output size for fidelity.
    const RASTER_SCALE: f32 = 2.0;
    elide_pdf::Pdf::open(document)
        .and_then(|pdf| pdf.observe(RASTER_SCALE))
        .map_err(pdf_error)
}
