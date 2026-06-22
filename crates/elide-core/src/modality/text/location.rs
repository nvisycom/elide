//! [`TextLocation`]: a byte range within text content.

use std::cmp::Ordering;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::ModalityLocation;

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
    #[must_use]
    pub const fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Whether the range is empty (zero length).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
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

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::*;

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
}
