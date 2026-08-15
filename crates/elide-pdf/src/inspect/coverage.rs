//! [`Coverage`]: whether inspection could account for the whole document.
//!
//! Inspection is fail-closed: it reports [`Full`](CoverageStatus::Full) coverage
//! only when nothing prevents it from reasoning about the entire document. Any
//! condition that leaves part of the document unexamined — encryption, a
//! retained prior revision, bytes after `%%EOF` — is recorded as a
//! [`Gap`](CoverageGap) and downgrades coverage to
//! [`Partial`](CoverageStatus::Partial), so a caller never mistakes an
//! incomplete inspection for a clean one.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Whether inspection accounted for the whole document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "lowercase")
)]
pub enum CoverageStatus {
    /// Nothing prevented inspecting the entire document.
    Full,
    /// One or more [`gaps`](Coverage::gaps) left part of the document
    /// unexamined.
    Partial,
}

/// Coverage of a document inspection: a status plus the specific gaps that
/// downgraded it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Coverage {
    /// [`Full`](CoverageStatus::Full) only when `gaps` is empty.
    pub status: CoverageStatus,
    /// The conditions that left part of the document unexamined.
    pub gaps: Vec<CoverageGap>,
}

impl Coverage {
    /// Build coverage from its gaps: [`Full`](CoverageStatus::Full) when there
    /// are none, otherwise [`Partial`](CoverageStatus::Partial).
    pub(crate) fn from_gaps(gaps: Vec<CoverageGap>) -> Self {
        let status = if gaps.is_empty() {
            CoverageStatus::Full
        } else {
            CoverageStatus::Partial
        };
        Self { status, gaps }
    }
}

/// A specific condition that left part of the document unexamined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "kebab-case")
)]
#[non_exhaustive]
pub enum CoverageGap {
    /// The document is encrypted; its object contents cannot be inspected.
    EncryptedDocument,
    /// The document retains superseded content from a prior incremental
    /// revision, or non-whitespace bytes after the final `%%EOF`. The current
    /// object graph was inspected; the retained bytes were not.
    RetainedDocumentBytes,
}
