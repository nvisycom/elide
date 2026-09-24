//! PDF loader: extract page text via [`Pdf`] and produce the handler.
//!
//! Redaction defaults to glyph deletion (keeps a selectable text layer). With
//! the `render` feature the loader carries a [`RasterMode`]; under
//! [`RasterMode::Always`] it observes pages for raster redaction (flatten to a
//! fresh image-only PDF) instead.
//!
//! [`RasterMode`]: super::RasterMode
//! [`RasterMode::Always`]: super::RasterMode::Always

use elide_codec::Loader;
use elide_codec::content::ContentData;
use elide_core::Result;
use elide_core::modality::text::Text;

#[cfg(feature = "render")]
use super::RasterMode;
use super::pdf_handler::{PdfHandler, PdfPage};
use crate::Pdf;
use crate::extract::Block;

/// Loader producing the [`PdfHandler`]: born-digital text extraction, plus the
/// optional page-rendering path (feature `render`).
#[derive(Debug, Default)]
pub(crate) struct PdfLoader {
    /// Whether redaction flattens pages to images (raster) instead of the
    /// default glyph deletion. Only meaningful with the `render` feature.
    #[cfg(feature = "render")]
    raster: RasterMode,
}

impl PdfLoader {
    /// A loader on the born-digital text path (no page rendering).
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A loader with an explicit [`RasterMode`] (feature `render`).
    ///
    /// [`RasterMode`]: super::RasterMode
    #[cfg(feature = "render")]
    pub(crate) fn with_raster(raster: RasterMode) -> Self {
        Self { raster }
    }
}

#[async_trait::async_trait]
impl Loader for PdfLoader {
    type Handler = PdfHandler;
    type Modality = Text;

    async fn decode(&self, content: ContentData) -> Result<PdfHandler> {
        let document = content.to_bytes();

        // `RasterMode::Always` (feature `render`): observe each page ,
        // its text comes from the renderer alongside its glyph geometry, so
        // redaction on encode fills the detected pixels and emits a fresh
        // image-only PDF (the flatten guarantee, no selectable text).
        #[cfg(feature = "render")]
        if self.raster.render_dpi().is_some() {
            let observations = observe_pages(&document)?;
            let pages = pages_from_blocks(
                observations
                    .iter()
                    .map(|o| Block::new(o.page, o.text.clone())),
            );
            return Ok(PdfHandler::raster(document, pages, observations));
        }

        // Default (`Auto`/`Never`, and the whole pure-Rust build): glyph
        // deletion. The page text comes from `extract`, the same content walk
        // `redact_text` uses, so a detection's character span maps to the glyphs
        // it drew. On encode the glyphs are deleted and annotations/metadata
        // stripped, keeping a selectable text layer.
        let pdf = Pdf::open(&document)?;
        // Extract once: `blocks` drive glyph deletion, and (in `Auto`) `issues`
        // name the textless pages to render. A second `extract()` would re-walk
        // every page and re-copy the embedded image bytes for nothing.
        let extraction = pdf.extract();
        let pages = pages_from_blocks(extraction.blocks);

        // `RasterMode::Auto` (feature `render` + an image codec): a textless
        // (scanned) page has no glyphs to delete, so render it and surface it as
        // an image part for the image pipeline to OCR and redact, while
        // born-digital pages keep glyph deletion. This is the Auto promise: text
        // where present, image where absent.
        #[cfg(all(feature = "render", feature = "image"))]
        if matches!(self.raster, RasterMode::Auto) {
            let scanned = scanned_pages(&pdf, &extraction.issues)?;
            if !scanned.is_empty() {
                return Ok(PdfHandler::text_auto(document, pages, scanned));
            }
        }

        Ok(PdfHandler::text(document, pages))
    }
}

/// Render each textless (`NeedsOcr`) page to a PNG, keyed by page number, for
/// the [`Container`](elide_codec::Container) to surface as an image part.
#[cfg(all(feature = "render", feature = "image"))]
fn scanned_pages(
    pdf: &Pdf,
    issues: &[crate::extract::Issue],
) -> Result<std::collections::BTreeMap<u32, bytes::Bytes>> {
    use crate::extract::IssueKind;

    // Which 1-based pages have no text layer.
    let textless: std::collections::BTreeSet<u32> = issues
        .iter()
        .filter(|issue| matches!(issue.kind, IssueKind::NeedsOcr))
        .map(|issue| issue.page)
        .collect();
    if textless.is_empty() {
        return Ok(std::collections::BTreeMap::new());
    }

    // Render only the textless pages, not the whole document: a mostly
    // born-digital PDF with a few scanned pages pays to rasterise just those.
    const RASTER_SCALE: f32 = 2.0;
    let rendered = pdf.render_pages(textless, RASTER_SCALE)?;
    Ok(rendered
        .into_iter()
        .map(|(number, page)| (number, bytes::Bytes::from(page.png)))
        .collect())
}

/// Assemble [`PdfPage`]s from the engine's per-page text [`Block`]s, assigning
/// each its start offset in the concatenated text stream.
///
/// Pages are separated by [`PAGE_SEPARATOR`] in the stream coordinate space: the
/// cumulative offset advances by each page's length *plus* the separator width,
/// so no detected span can straddle two pages (which encode would then drop).
fn pages_from_blocks(blocks: impl IntoIterator<Item = Block>) -> Vec<PdfPage> {
    let mut pages = Vec::new();
    let mut offset = 0usize;
    for block in blocks {
        let text = block.text.to_string();
        let len = text.len();
        pages.push(PdfPage {
            number: block.page,
            text,
            start: offset,
        });
        offset += len + PAGE_SEPARATOR.len();
    }
    pages
}

/// The gap inserted between consecutive pages in the concatenated stream so a
/// detection cannot span a page boundary.
const PAGE_SEPARATOR: &str = "\n";

/// Observe every page for raster redaction: render it to pixels and extract its
/// text-layer glyph geometry, so the page text and glyph boxes share one
/// coordinate system.
#[cfg(feature = "render")]
fn observe_pages(document: &[u8]) -> Result<Vec<crate::render::PageObservation>> {
    // A default render scale; higher scales trade output size for fidelity.
    const RASTER_SCALE: f32 = 2.0;
    Pdf::open(document).and_then(|pdf| pdf.observe(RASTER_SCALE))
}
