//! DOCX scenarios and their shared fixture.
//!
//! The fixture is a real Word-authored document: its PII spans the body
//! (`word/document.xml`), a page header (`word/header3.xml`), and an
//! external hyperlink `mailto:` target in `word/_rels/document.xml.rels`,
//! so redaction must reach every text-bearing part and the relationship
//! targets, not just the body, while the styles, theme, and
//! content-types parts pass through unchanged. More docx samples plug in
//! alongside this one.

// Each scenario module uses a different subset of these.
#![allow(dead_code)]

mod docprops;
mod rebuilt_report;
mod redaction;
mod split_runs;

use crate::support::fixture::Fixture;

/// The real Word-authored sample driving the docx scenarios.
pub const FIXTURE: Fixture = Fixture {
    path: concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/testdata/docx/body_header_hyperlink.docx"
    ),
    source: include_bytes!("../testdata/docx/body_header_hyperlink.docx"),
    extension: "docx",
};

pub const BODY_PART: &str = "word/document.xml";
pub const HEADER_PART: &str = "word/header3.xml";
pub const RELS_PART: &str = "word/_rels/document.xml.rels";

/// Every PII value `docx/body_header_hyperlink.docx` carries; a redacted document must contain none of
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
