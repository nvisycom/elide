//! The [`Text`] modality — plain or structured text addressed by byte
//! ranges.

use std::cmp::Ordering;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{Modality, ModalityData, ModalityLocation, ModalityReplacement};

/// A run of text — the payload a text recognizer inspects, or the value
/// sliced out at an entity's location for an operator.
///
/// Held as a [`HipStr`] so short values inline and longer ones share a
/// refcounted buffer, making cheap clones when one payload is passed to
/// several recognizers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct TextData {
    /// The text content.
    pub text: HipStr<'static>,
}

impl TextData {
    /// Wrap a string as text data.
    pub fn new(text: impl Into<HipStr<'static>>) -> Self {
        Self { text: text.into() }
    }

    /// The text as a string slice.
    pub fn as_str(&self) -> &str {
        self.text.as_str()
    }
}

impl ModalityData for TextData {}

/// A half-open `[start, end)` byte range within text content.
///
/// Ordering and overlap consider only `(start, end)`; the optional
/// page number is carried for codecs that page their text but does not
/// affect comparison.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TextLocation {
    /// Byte offset where the range starts.
    pub start: usize,
    /// Byte offset where the range ends (exclusive).
    pub end: usize,
    /// 1-based page number, when known.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub page: Option<u32>,
}

impl TextLocation {
    /// A location covering `start..end`, page unset.
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            page: None,
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

/// What a text operator produces: a substitution or a removal.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum TextReplacement {
    /// Replace the span with this value.
    Substituted(String),
    /// Remove the span entirely.
    Removed,
}

impl TextReplacement {
    /// A substitution with the given value.
    pub fn substituted(value: impl Into<String>) -> Self {
        Self::Substituted(value.into())
    }

    /// The replacement value, or `None` for a removal.
    pub fn value(&self) -> Option<&str> {
        match self {
            Self::Substituted(value) => Some(value),
            Self::Removed => None,
        }
    }
}

impl ModalityReplacement for TextReplacement {}

/// The text modality: data is [`TextData`], locations are
/// [`TextLocation`] byte ranges, replacements are [`TextReplacement`].
#[derive(Debug, Clone, Copy)]
pub struct Text;

impl Modality for Text {
    type Data = TextData;
    type Location = TextLocation;
    type Replacement = TextReplacement;

    const NAME: &'static str = "text";
}
