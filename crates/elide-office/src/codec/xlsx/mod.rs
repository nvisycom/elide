//! XLSX codec: a workbook is a [`Document`] whose body is a
//! [`Stream<Tabular>`] of cells, plus a text stream over its content outside the
//! cells (comments, drawings, charts, external hyperlink targets), a [`Blob`]
//! per embedding, and a metadata stream per document-property part.
//!
//! A cell holds text, so a `TabularReplacement`'s cell treatment applies
//! through the shared text-redaction helper. Only the *location* is tabular: a
//! `(sheet, row, column)` address with an optional intra-cell byte range. The
//! [`XlsxDocument`](document::XlsxDocument) re-packs the workbook
//! byte-faithfully, de-sharing any shared-string cell that a redaction changed so
//! other cells keep the pooled value, and splicing the redacted text and folding
//! the redacted embedding/property parts in alongside.
//!
//! [`Document`]: elide_codec::Document
//! [`Stream<Tabular>`]: elide_codec::Stream
//! [`Blob`]: elide_codec::Blob

mod document;
mod loader;
mod state;
mod stream;

use elide_codec::{Format, FormatId};

use self::loader::XlsxDocumentLoader;

/// Stable [`FormatId`] for the XLSX codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.tabular.xlsx");

/// The non-cell text stream's document-local id (not a zip entry), beside the
/// cell body stream's shared [`BODY_PART_ID`](crate::codec::ooxml::BODY_PART_ID).
const TEXT_PART_ID: &str = "text";

/// The XLSX [`Format`]: Excel workbooks, resolved by `.xlsx`.
pub fn format() -> Format {
    Format::with_document_loader(FORMAT_ID, XlsxDocumentLoader)
        .with_extensions(["xlsx"])
        .with_content_types(["application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"])
}
