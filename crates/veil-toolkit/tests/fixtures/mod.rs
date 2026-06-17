//! Shared test fixture: a minimal `Text` modality modelled on
//! `nvisy-runtime`'s, the kind a real `veil-text` crate would provide.
#![allow(dead_code)] // a fixture exposes more API than any one test uses

use std::cmp::Ordering;

use veil_core::modality::{Modality, ModalityData, ModalityLocation, ModalityReplacement};

/// Per-call text payload — the content recognizers inspect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextData(pub String);

impl ModalityData for TextData {}

/// A half-open `[start, end)` byte range within text content.
///
/// Modelled on runtime's `TextLocation`: the core span plus the
/// optional surrounding context window and page number a codec may
/// attach. Ordering and overlap consider only `(start, end)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextLocation {
    /// Byte offset where the range starts.
    pub start: usize,
    /// Byte offset where the range ends (exclusive).
    pub end: usize,
    /// Surrounding context window for redaction, when known.
    pub context: Option<(usize, usize)>,
    /// 1-based page number, when known.
    pub page_number: Option<u32>,
}

impl TextLocation {
    /// A location covering `start..end`, optional fields unset.
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            context: None,
            page_number: None,
        }
    }

    /// Byte length of the range (`end - start`).
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Whether the range is empty (zero length).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl ModalityLocation for TextLocation {
    fn overlaps(&self, other: &Self) -> bool {
        self.start < other.end && other.start < self.end
    }

    fn span_cmp(&self, other: &Self) -> Ordering {
        self.len().cmp(&other.len())
    }
}

/// What a text anonymizer produces: a substitution or a removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextReplacement {
    /// Replace the span with this value.
    Substituted(String),
    /// Remove the span entirely.
    Removed,
}

impl ModalityReplacement for TextReplacement {}

/// The text modality marker, binding the data/location/replacement types.
#[derive(Debug, Clone, Copy)]
pub struct Text;

impl Modality for Text {
    type Data = TextData;
    type Location = TextLocation;
    type Replacement = TextReplacement;
    const NAME: &'static str = "text";
}
