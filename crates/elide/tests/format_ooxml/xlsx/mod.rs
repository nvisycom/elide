//! XLSX scenarios and their shared fixtures.
//!
//! The primary fixture is a real Excel-authored workbook whose cell text
//! lives in the shared-string table; redacting a shared-string cell
//! de-shares it (the cell becomes an inline string) and blanks any pooled
//! value left with no reference — so no PII survives anywhere in the
//! output package, not even as an orphaned shared string. A second fixture
//! carries non-cell PII (a cell comment, a drawing's text) surfaced as XML
//! container parts.

// Each scenario module uses a different subset of these.
#![allow(dead_code)]

mod comments;
mod redaction;

use crate::support::pipeline::Fixture;

/// The primary Excel-authored workbook: PII in shared-string cells.
pub const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/sample.xlsx"),
    source: include_bytes!("../../testdata/sample.xlsx"),
    extension: "xlsx",
};

/// A workbook whose PII lives outside the cell grid: an email in a cell
/// comment, a phone in a drawing's text, and an email in an external hyperlink
/// `Target` in the workbook's relationships.
pub const FIXTURE_NON_CELL: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/sample2.xlsx"),
    source: include_bytes!("../../testdata/sample2.xlsx"),
    extension: "xlsx",
};

/// The non-cell PII carried by [`FIXTURE_NON_CELL`].
pub const NON_CELL_PII: &[&str] = &["carol@example.com", "+1 (510) 555-0199", "dave@example.com"];
