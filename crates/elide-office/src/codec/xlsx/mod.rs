//! XLSX codec on the parts model: a workbook is a [`Document`] whose body is a
//! [`Stream<Tabular>`] of cells, plus a text stream over its content outside the
//! cells (comments, drawings, charts, external hyperlink targets) and a
//! [`Blob`] per embedding and per document-property part.
//!
//! A cell holds text, so a `TabularReplacement`'s cell treatment applies
//! through the shared text-redaction helper. Only the *location* is tabular: a
//! `(sheet, row, column)` address with an optional intra-cell byte range. The
//! [`XlsxRecombine`](recombine::XlsxRecombine) re-packs the workbook
//! byte-faithfully, de-sharing any shared-string cell that a redaction changed so
//! other cells keep the pooled value, and splicing the redacted text and folding
//! the redacted embedding/property parts in alongside.
//!
//! [`Document`]: elide_codec::Document
//! [`Stream<Tabular>`]: elide_codec::Stream
//! [`Blob`]: elide_codec::DocumentPart::Blob

mod loader;
mod recombine;
mod state;
mod stream;

use elide_codec::{Format, FormatId};

use self::loader::XlsxDocumentLoader;

/// Stable [`FormatId`] for the XLSX codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.tabular.xlsx");

/// The cell body stream's part id; excluded from the re-packed part
/// replacements.
const BODY_PART_ID: &str = "body";

/// The non-cell text stream's part id; excluded from the re-packed part
/// replacements.
const TEXT_PART_ID: &str = "text";

/// [`Format`] descriptor registered into `FormatRegistry`.
pub fn format() -> Format {
    Format::with_document_loader(FORMAT_ID.clone(), XlsxDocumentLoader)
        .with_extensions(["xlsx"])
        .with_content_types(["application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"])
}

/// One extracted cell: its sheet, zero-based coordinates, and current text.
#[derive(Debug, Clone)]
pub(crate) struct XlsxCell {
    /// Display name of the sheet the cell is on.
    pub(crate) sheet: String,
    /// Zero-based row index.
    pub(crate) row: u32,
    /// Zero-based column index.
    pub(crate) column: u32,
    /// The cell's text, mutated in place as redactions apply.
    pub(crate) text: String,
}
