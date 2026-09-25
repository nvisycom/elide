//! Codec adapter: the PDF format on the parts model.
//!
//! Binds this crate's PDF engine to the `elide-codec`
//! [`DocumentLoader`](elide_codec::DocumentLoader)/[`Stream`](elide_codec::Stream)/
//! [`Recombine`](elide_codec::Recombine) contracts: a document is a page-text
//! body [`Stream<Text>`](pdf_stream::PdfStream) plus, on the glyph-deletion path,
//! a [`Blob`](elide_codec::DocumentPart::Blob) per redactable embedded image
//! XObject and per textless (scanned) page. A redaction on the body stream is
//! recorded on the shared [`PdfState`](pdf_state::PdfState) and applied by
//! [`PdfRecombine`](pdf_recombine::PdfRecombine) per the document's [`RedactMode`]:
//!
//! - **Glyph deletion** (default, pure-Rust): the detected glyphs are deleted
//!   from the content streams and annotations/metadata stripped, keeping a
//!   selectable text layer with the detected spans gone ([`Pdf::redact_text`]).
//! - **Raster** (feature `render`, [`RasterMode::Always`]): the page text comes
//!   from [`Pdf::observe`] alongside its glyph geometry, so a redaction's span
//!   maps to pixel boxes; encode fills them and emits a fresh image-only PDF, the
//!   text layer is gone entirely.
//!
//! [`pdf_format`] decodes on the glyph-deletion path; with the `render` feature
//! [`pdf_format_with`] takes an explicit [`RasterMode`].
//!
//! [`Pdf::redact_text`]: crate::document::Pdf::redact_text
//! [`Pdf::observe`]: crate::document::Pdf::observe
//! [`RasterMode::Always`]: crate::primitive::RasterMode::Always

mod pdf_loader;
mod pdf_recombine;
mod pdf_state;
mod pdf_stream;

use elide_codec::{Format, FormatId};

use self::pdf_loader::PdfDocumentLoader;
#[cfg(feature = "render")]
use crate::primitive::RasterMode;
#[cfg(feature = "render")]
use crate::redact::Detection;
#[cfg(feature = "render")]
use crate::render::PageObservation;

/// Stable [`FormatId`] for the PDF codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.document.pdf");

/// The body stream's part id; excluded from the blob-part fold on assemble.
const BODY_PART_ID: &str = "body";

/// The gap inserted between consecutive pages in the concatenated stream so a
/// detection cannot span a page boundary.
const PAGE_SEPARATOR: &str = "\n";

/// [`Format`] descriptor registered into `FormatRegistry`.
///
/// Decodes on the glyph-deletion redaction path. To flatten pages to images
/// instead, build the format with [`pdf_format_with`] and `RasterMode::Always`.
pub fn pdf_format() -> Format {
    Format::with_document_loader(FORMAT_ID.clone(), PdfDocumentLoader::new())
        .with_extensions(["pdf"])
        .with_content_types(["application/pdf"])
}

/// [`Format`] descriptor with an explicit [`RasterMode`].
///
/// Under [`RasterMode::Always`] redaction flattens every page to an image
/// (a fresh image-only PDF); [`Auto`](RasterMode::Auto) and
/// [`Never`](RasterMode::Never) use the default glyph-deletion path.
///
/// [`RasterMode::Always`]: crate::primitive::RasterMode::Always
#[cfg(feature = "render")]
#[cfg_attr(docsrs, doc(cfg(feature = "render")))]
pub fn pdf_format_with(raster: RasterMode) -> Format {
    Format::with_document_loader(FORMAT_ID.clone(), PdfDocumentLoader::with_raster(raster))
        .with_extensions(["pdf"])
        .with_content_types(["application/pdf"])
}

/// One page's text and where it sits in the concatenated text stream.
#[derive(Debug, Clone)]
pub(crate) struct PdfPage {
    /// 1-based page number.
    pub(crate) number: u32,
    /// The page's current (possibly redacted) text.
    pub(crate) text: String,
    /// Start offset of this page in the concatenated stream.
    pub(crate) start: usize,
}

/// How the document's recorded redactions are applied on encode.
#[derive(Debug, Default)]
pub(crate) enum RedactMode {
    /// Glyph deletion via [`Pdf::redact_text`](crate::document::Pdf::redact_text): the
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
