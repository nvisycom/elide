//! [`TextLocation`]: a byte range within text content.

use std::cmp::Ordering;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::{ModalityLocation, Overlap};

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
    fn overlap(&self, other: &Self) -> Overlap {
        if self.start >= other.end || other.start >= self.end {
            return Overlap::Disjoint;
        }
        if self.start <= other.start && other.end <= self.end {
            return Overlap::Contains;
        }
        if other.start <= self.start && self.end <= other.end {
            return Overlap::ContainedBy;
        }
        let inter = self.end.min(other.end) - self.start.max(other.start);
        let union = self.end.max(other.end) - self.start.min(other.start);
        Overlap::Crossing {
            iou: inter as f32 / union as f32,
        }
    }

    fn union(&self, other: &Self) -> Option<Self> {
        // A single byte range can't span two pages; require agreement.
        if self.page != other.page {
            return None;
        }
        Some(Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
            page: self.page,
        })
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
    fn overlap_classifies_the_relationship() {
        let a = TextLocation::new(0, 10);
        // Disjoint.
        assert_eq!(a.overlap(&TextLocation::new(10, 20)), Overlap::Disjoint);
        // Nesting, both directions.
        assert_eq!(a.overlap(&TextLocation::new(2, 8)), Overlap::Contains);
        assert_eq!(TextLocation::new(2, 8).overlap(&a), Overlap::ContainedBy);
        // Identical extent reads as containment.
        assert_eq!(a.overlap(&a), Overlap::Contains);
        // Crossing, with an IoU measure.
        let Overlap::Crossing { iou } = a.overlap(&TextLocation::new(5, 15)) else {
            panic!("expected crossing");
        };
        assert!((iou - 5.0 / 15.0).abs() < 1e-6);
    }

    #[test]
    fn union_is_the_bounding_range() {
        let a = TextLocation::new(0, 5);
        let b = TextLocation::new(3, 12);
        let u = a.union(&b).expect("same page");
        assert_eq!((u.start, u.end), (0, 12));
        // Reflexive.
        assert_eq!(a.union(&a), Some(a.clone()));
    }

    #[test]
    fn union_requires_same_page() {
        let a = TextLocation {
            start: 0,
            end: 5,
            page: Some(1),
        };
        let b = TextLocation {
            start: 3,
            end: 12,
            page: Some(2),
        };
        // A single byte range can't span two pages.
        assert_eq!(a.union(&b), None);
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
