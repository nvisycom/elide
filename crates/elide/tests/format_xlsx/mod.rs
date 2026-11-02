//! XLSX scenarios and their shared fixtures.
//!
//! The primary fixture is a real Excel-authored workbook whose cell text
//! lives in the shared-string table; redacting a shared-string cell
//! de-shares it (the cell becomes an inline string) and blanks any pooled
//! value left with no reference, so no PII survives anywhere in the
//! output package, not even as an orphaned shared string. A second fixture
//! carries non-cell PII (a cell comment, a drawing's text) surfaced as XML
//! container parts.

// Each scenario module uses a different subset of these.
#![allow(dead_code)]

mod codec;
mod comments;
mod docprops;
mod redaction;

use crate::support::fixture::Fixture;

/// The primary Excel-authored workbook: PII in shared-string cells.
pub const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/xlsx/shared_string_cells.xlsx"
    ),
    source: include_bytes!("../testdata/xlsx/shared_string_cells.xlsx"),
    extension: "xlsx",
};

/// A workbook whose PII lives outside the cell grid: an email in a cell
/// comment, a phone in a drawing's text, and an email in an external hyperlink
/// `Target` in the workbook's relationships.
pub const FIXTURE_NON_CELL: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/xlsx/non_cell_text.xlsx"
    ),
    source: include_bytes!("../testdata/xlsx/non_cell_text.xlsx"),
    extension: "xlsx",
};

/// The non-cell PII carried by [`FIXTURE_NON_CELL`].
pub const NON_CELL_PII: &[&str] = &["carol@example.com", "+1 (510) 555-0199", "dave@example.com"];

/// Every PII value `xlsx/shared_string_cells.xlsx` carries; a redacted document must contain none of
/// them. All values are synthetic: `@example.com` addresses, a Luhn-valid test
/// card, an RFC 5737 documentation IP.
pub const FIXTURE_PII: &[&str] = &[
    "alice.johnson@example.com",
    "bob.smith@example.com",
    "+1 (415) 555-0142",
    "+1 (510) 555-0199",
    "4111 1111 1111 1111",
    "GB29 NWBK 6016 1331 9268 19",
    "123-45-6789",
    "192.168.1.42",
];
