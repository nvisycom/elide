//! PPTX scenarios and their fixture.
//!
//! A presentation's user text lives as DrawingML `a:t` runs in its slides
//! (and notes, masters, comments). The handler redacts that text as it
//! would a DOCX body, and the package re-packs byte-faithfully with only
//! the redacted parts changed.

// Each scenario module uses a different subset of these.
#![allow(dead_code)]

mod redaction;

use crate::support::pipeline::Fixture;

/// A real presentation whose slide carries PII in its `a:t` text runs.
pub const FIXTURE: Fixture = Fixture {
    path: concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata/sample.pptx"),
    source: include_bytes!("../../testdata/sample.pptx"),
    extension: "pptx",
};

/// The PII in the slide's text runs: an email and a phone.
pub const PII: &[&str] = &["alice@example.com", "+1 (510) 555-0199"];
