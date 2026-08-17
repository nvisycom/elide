//! Scaffolding shared across the OOXML formats.
//!
//! DOCX, XLSX, and PPTX all package their parts through the same OPC
//! engine, so the sanctioned way to inspect a redacted package — read
//! a part back out, sweep every text-bearing part for leaked PII — is one
//! surface, provided by [`PipelineOutcome`] in `support`. What lives here
//! is the content the formats have in common: the synthetic PII set the
//! shared `sample.*` fixtures carry.
//!
//! [`PipelineOutcome`]: crate::support::pipeline::PipelineOutcome

// Each format's scenarios use a different subset of the shared items.
#![allow(dead_code)]

pub mod docx;
pub mod pptx;
pub mod xlsx;

/// Every PII value the shared `sample.docx` / `sample.xlsx` fixtures carry,
/// across their body, header, and relationship parts. A redacted package
/// must contain none of these. All values are synthetic: `@example.com`
/// addresses, a Luhn-valid test card, an RFC 5737 documentation IP.
pub const SHARED_PII: &[&str] = &[
    "alice.johnson@example.com",
    "bob.smith@example.com",
    "+1 (415) 555-0142",
    "+1 (510) 555-0199",
    "4111 1111 1111 1111",
    "GB29 NWBK 6016 1331 9268 19",
    "123-45-6789",
    "192.168.1.42",
];
