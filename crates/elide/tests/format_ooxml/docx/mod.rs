//! DOCX scenarios and their shared fixture.
//!
//! The fixture is a real Word-authored document: its PII spans the body
//! (`word/document.xml`), a page header (`word/header3.xml`), and an
//! external hyperlink `mailto:` target in `word/_rels/document.xml.rels`,
//! so redaction must reach every text-bearing part and the relationship
//! targets — not just the body — while the styles, theme, and
//! content-types parts pass through unchanged. More docx samples plug in
//! alongside this one.

// Each scenario module uses a different subset of these.
#![allow(dead_code)]

mod rebuilt_report;
mod redaction;

use crate::support::pipeline::Fixture;

/// The real Word-authored sample driving the docx scenarios.
pub const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/sample.docx"),
    source: include_bytes!("../../testdata/sample.docx"),
    extension: "docx",
};

pub const BODY_PART: &str = "word/document.xml";
pub const HEADER_PART: &str = "word/header3.xml";
pub const RELS_PART: &str = "word/_rels/document.xml.rels";
