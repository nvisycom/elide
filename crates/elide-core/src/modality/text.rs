//! [`Text`] modality: plain or structured text addressed by byte ranges.

use std::cmp::Ordering;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{Modality, ModalityData, ModalityLocation, ModalityReplacement};

/// Run of text.
///
/// Either the payload a text recognizer inspects, or the value sliced out
/// at an entity's location for an operator.
///
/// Held as a [`HipStr`] so short values inline and longer ones share a
/// refcounted buffer, making cheap clones when one payload is passed to
/// several recognizers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct TextData {
    /// Text content.
    pub text: HipStr<'static>,
}

impl TextData {
    /// Wrap a string as text data.
    pub fn new(text: impl Into<HipStr<'static>>) -> Self {
        Self { text: text.into() }
    }

    /// Text as a string slice.
    pub fn as_str(&self) -> &str {
        self.text.as_str()
    }
}

impl ModalityData for TextData {}

/// Half-open `[start, end)` byte range within text content.
///
/// Ordering and overlap consider only `(start, end)`; the optional page
/// number is carried for codecs that page their text but does not affect
/// comparison.
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
    /// Location covering `start..end`, page unset.
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

    fn position_cmp(&self, other: &Self) -> Ordering {
        // Reading order: page first (unpaged sorts as page 0), then by
        // start offset, then by end so a shorter span at the same start
        // sorts before a longer one.
        self.page
            .unwrap_or(0)
            .cmp(&other.page.unwrap_or(0))
            .then(self.start.cmp(&other.start))
            .then(self.end.cmp(&other.end))
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
    /// Substitution with the given value.
    pub fn substituted(value: impl Into<String>) -> Self {
        Self::Substituted(value.into())
    }

    /// Replacement value, or `None` for a removal.
    pub fn value(&self) -> Option<&str> {
        match self {
            Self::Substituted(value) => Some(value),
            Self::Removed => None,
        }
    }
}

impl ModalityReplacement for TextReplacement {}

/// Text modality: data is [`TextData`], locations are
/// [`TextLocation`] byte ranges, replacements are [`TextReplacement`].
#[derive(Debug, Clone, Copy)]
pub struct Text;

impl Modality for Text {
    type Data = TextData;
    type Location = TextLocation;
    type Replacement = TextReplacement;

    const NAME: &'static str = "text";
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::*;
    use crate::redaction::Redactions;

    #[test]
    fn position_cmp_orders_by_start_then_end() {
        let a = TextLocation::new(0, 5);
        let b = TextLocation::new(3, 8);
        let c = TextLocation::new(3, 4);
        assert_eq!(a.position_cmp(&b), Ordering::Less);
        // Same start: shorter end sorts first.
        assert_eq!(c.position_cmp(&b), Ordering::Less);
        assert_eq!(b.position_cmp(&a), Ordering::Greater);
    }

    #[test]
    fn position_cmp_orders_pages_before_offsets() {
        let early_page = TextLocation {
            start: 100,
            end: 110,
            page: Some(1),
        };
        let late_page = TextLocation {
            start: 0,
            end: 5,
            page: Some(2),
        };
        // Page 1 sorts before page 2 even with a larger offset.
        assert_eq!(early_page.position_cmp(&late_page), Ordering::Less);
    }

    #[test]
    fn span_cmp_is_extent_not_position() {
        let short_late = TextLocation::new(10, 12);
        let long_early = TextLocation::new(0, 9);
        // Positionally the early one is first...
        assert_eq!(long_early.position_cmp(&short_late), Ordering::Less);
        // ...but by extent it is the larger span.
        assert_eq!(long_early.span_cmp(&short_late), Ordering::Greater);
    }

    #[test]
    fn sort_by_position_orders_in_place() {
        let mut batch: Redactions<Text> = Redactions::new();
        // Pushed out of order.
        batch.push(TextLocation::new(20, 25), TextReplacement::Removed);
        batch.push(TextLocation::new(0, 5), TextReplacement::Removed);
        batch.push(TextLocation::new(10, 15), TextReplacement::Removed);

        batch.sort_by_position();

        let starts: Vec<usize> = batch.iter().map(|(loc, _)| loc.start).collect();
        assert_eq!(starts, [0, 10, 20]);
    }
}
