//! The codec adapter for PDF: the format, decoded for glyph-level redaction.
//!
//! Binds this crate's PDF engine to the `elide-codec`
//! [`DocumentLoader`](elide_codec::DocumentLoader)/[`Stream`](elide_codec::Stream)/
//! [`Native`](elide_codec::Native) contracts: a document is a page-text body
//! `Stream<Text>` (`PdfStream`) plus, on the glyph-deletion path, a
//! [`Blob`](elide_codec::Blob) per redactable embedded image XObject and per
//! textless (scanned) page. A redaction on the body stream is recorded on its
//! `PdfState` and applied by `PdfDocument` per the document's `RedactMode`:
//!
//! - **Glyph deletion** (default, pure-Rust): the detected glyphs are deleted
//!   from the content streams and annotations/metadata stripped, keeping a
//!   selectable text layer with the detected spans gone (`Pdf::apply`).
//! - **Raster** (feature `render`, [`RasterMode::Always`]): the page text comes
//!   from `Pdf::observe` alongside its glyph geometry, so a redaction's span
//!   maps to pixel boxes; encode fills them and emits a fresh image-only PDF, the
//!   text layer is gone entirely.
//!
//! [`pdf_format`] decodes on the glyph-deletion path; with the `render` feature
//! [`pdf_format_with`] takes an explicit [`RasterMode`].
//!
//! [`RasterMode`]: crate::primitive::RasterMode
//! [`RasterMode::Always`]: crate::primitive::RasterMode::Always

mod document;
mod loader;
mod part_id;
mod state;
mod stream;

use std::sync::Arc;

use elide_codec::{Format, FormatId};

use self::loader::PdfDocumentLoader;
#[cfg(feature = "render")]
use crate::primitive::RasterMode;
#[cfg(feature = "render")]
use crate::render::PageObservation;
#[cfg(feature = "render")]
use crate::text::Detection;
use crate::text::PageGeometry;

/// The stable [`FormatId`] of the PDF format, `elide.document.pdf`.
pub const FORMAT_ID: FormatId = FormatId::new("elide.document.pdf");

/// The body stream's part id; excluded from the blob-part fold on assemble.
const BODY_PART_ID: &str = "body";

/// The PDF [`Format`], decoding pages for glyph-deletion redaction.
///
/// To flatten pages to images
/// instead, build the format with [`pdf_format_with`] and `RasterMode::Always`.
pub fn pdf_format() -> Format {
    Format::with_document_loader(FORMAT_ID, PdfDocumentLoader::new())
        .with_extensions(["pdf"])
        .with_content_types(["application/pdf"])
}

/// The PDF [`Format`] with an explicit [`RasterMode`], to flatten pages.
///
/// Under [`RasterMode::Always`] redaction flattens every page to an image
/// (a fresh image-only PDF); [`Auto`](RasterMode::Auto) and
/// [`Never`](RasterMode::Never) use the default glyph-deletion path.
///
/// [`RasterMode::Always`]: crate::primitive::RasterMode::Always
#[cfg(feature = "render")]
#[cfg_attr(docsrs, doc(cfg(feature = "render")))]
pub fn pdf_format_with(raster: RasterMode) -> Format {
    Format::with_document_loader(FORMAT_ID, PdfDocumentLoader::with_raster(raster))
        .with_extensions(["pdf"])
        .with_content_types(["application/pdf"])
}

/// One page's text and its rendered glyph geometry. Each page is addressed in
/// its own page-local byte space; there is no document-wide offset.
#[derive(Debug, Clone)]
pub(crate) struct PdfPage {
    /// 1-based page number.
    pub(crate) number: u32,
    /// The page's current (possibly redacted) text.
    pub(crate) text: String,
    /// The page's rendered glyph geometry, for resolving a detected character
    /// span to page-normalized highlight rectangles. `None` when geometry was
    /// not gathered (a page the walk could not read). Shared through an [`Arc`]
    /// so a lookup per detected entity is a cheap refcount bump, not a deep clone
    /// of the page's whole glyph list.
    pub(crate) geometry: Option<Arc<PageGeometry>>,
}

/// How the document's recorded redactions are applied on encode.
#[derive(Debug, Default)]
pub(crate) enum RedactMode {
    /// Glyph deletion via [`Pdf::apply`](crate::document::Pdf::apply): the
    /// detected glyphs are removed and annotations/metadata stripped, keeping a
    /// selectable text layer. The default pure-Rust redaction path.
    #[default]
    GlyphDelete,
    /// Raster redaction (feature `render`): the page text comes from
    /// [`Pdf::observe`](crate::document::Pdf::observe), so a redaction's span maps directly
    /// to glyph pixel boxes; encode fills them and emits a fresh image-only PDF.
    #[cfg(feature = "render")]
    Raster {
        /// Per-page observations (text, glyph boxes, pixels) from `observe`.
        observations: Vec<PageObservation>,
        /// Recorded detections (page + character span), applied at encode.
        detections: Vec<Detection>,
    },
}
