//! [`TextLocation`]: a byte range within text content.

use std::cmp::Ordering;
use std::ops::Range;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::SourceRef;
use crate::modality::{ModalityLocation, Overlap};

/// Half-open `[start, end)` byte range within text content.
///
/// Ordering and overlap consider only the `range`; the optional page number is
/// carried for codecs that page their text but does not affect comparison.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TextLocation {
    /// Byte range within the (decoded) text content.
    #[cfg_attr(feature = "schema", schemars(with = "Range<usize>"))]
    pub range: Range<usize>,
    /// 1-based page number, when known.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub page: Option<u32>,
    /// The exact raw source ranges this decoded span came from, for codecs whose
    /// decoded text differs from the source (XML/HTML/DOCX, where entities are
    /// decoded; JSON, where `\"` / `\uXXXX` escapes collapse). Empty when the
    /// source equals the decoded text (plain text, CSV) or the format has no
    /// byte-source coordinate (rendered/scanned formats).
    ///
    /// Usually one range; a reconciled span that fused several source runs (or a
    /// span crossing an escape) carries several, kept distinct rather than merged
    /// across gaps. Sorted, deduplicated.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub source: Vec<SourceRef>,
}

impl TextLocation {
    /// Location covering `start..end`, page unset and no source range.
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            range: start..end,
            page: None,
            source: Vec::new(),
        }
    }

    /// Set the 1-based page number, consuming and returning `self`.
    #[must_use]
    pub fn with_page(mut self, page: Option<u32>) -> Self {
        self.page = page;
        self
    }

    /// Attach the exact raw source references, consuming and returning `self`.
    /// Normalizes them (sorted, deduplicated).
    #[must_use]
    pub fn with_source(mut self, source: impl IntoIterator<Item = SourceRef>) -> Self {
        self.source = source.into_iter().collect();
        SourceRef::normalize(&mut self.source);
        self
    }

    /// Byte length of the range.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.range.end.saturating_sub(self.range.start)
    }

    /// Whether the range is empty (zero length).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl ModalityLocation for TextLocation {
    fn overlap(&self, other: &Self) -> Overlap {
        let (a, b) = (&self.range, &other.range);
        if a.start >= b.end || b.start >= a.end {
            return Overlap::Disjoint;
        }
        if a.start <= b.start && b.end <= a.end {
            return Overlap::Contains;
        }
        if b.start <= a.start && a.end <= b.end {
            return Overlap::ContainedBy;
        }
        let inter = a.end.min(b.end) - a.start.max(b.start);
        let union = a.end.max(b.end) - a.start.min(b.start);
        Overlap::Crossing {
            iou: inter as f32 / union as f32,
        }
    }

    fn union(&self, other: &Self) -> Option<Self> {
        // A single byte range can't span two pages; require agreement.
        if self.page != other.page {
            return None;
        }
        // The fused decoded span covers both operands' source ranges. Keep them
        // as distinct ranges — merged runs and escape-split spans are genuinely
        // non-contiguous in the raw bytes — normalized so the result is
        // order-independent.
        let mut source = self.source.clone();
        source.extend_from_slice(&other.source);
        SourceRef::normalize(&mut source);
        Some(Self {
            range: self.range.start.min(other.range.start)..self.range.end.max(other.range.end),
            page: self.page,
            source,
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
            .then(self.range.start.cmp(&other.range.start))
            .then(self.range.end.cmp(&other.range.end))
    }

    fn hash(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(self.range.start as u64).to_le_bytes());
        bytes.extend_from_slice(&(self.range.end as u64).to_le_bytes());
        match self.page {
            Some(page) => {
                bytes.push(1);
                bytes.extend_from_slice(&page.to_le_bytes());
            }
            None => bytes.push(0),
        }
        // Fold the raw source refs in (count, then each range and part), so
        // tampering with the source pointer breaks the chain. `source` is
        // normalized, so the byte sequence is stable regardless of how the refs
        // were accumulated.
        bytes.extend_from_slice(&(self.source.len() as u64).to_le_bytes());
        for src in &self.source {
            bytes.extend_from_slice(&(src.range.start as u64).to_le_bytes());
            bytes.extend_from_slice(&(src.range.end as u64).to_le_bytes());
            match &src.part {
                Some(part) => {
                    bytes.push(1);
                    bytes.extend_from_slice(&(part.len() as u64).to_le_bytes());
                    bytes.extend_from_slice(part.as_bytes());
                }
                None => bytes.push(0),
            }
        }
        bytes
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
        let early_page = TextLocation::new(100, 110).with_page(Some(1));
        let late_page = TextLocation::new(0, 5).with_page(Some(2));
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
        assert_eq!(u.range, 0..12);
        // Reflexive.
        assert_eq!(a.union(&a), Some(a.clone()));
    }

    #[test]
    fn union_requires_same_page() {
        let a = TextLocation::new(0, 5).with_page(Some(1));
        let b = TextLocation::new(3, 12).with_page(Some(2));
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

    fn span(start: usize, end: usize) -> SourceRef {
        SourceRef::new(start..end)
    }

    #[test]
    fn with_source_normalizes_sorted_and_deduped() {
        let loc = TextLocation::new(0, 10).with_source([span(5, 8), span(1, 3), span(5, 8)]);
        // Sorted by start, exact duplicate dropped.
        assert_eq!(loc.source, vec![span(1, 3), span(5, 8)]);
    }

    #[test]
    fn union_concatenates_and_normalizes_source() {
        let a = TextLocation::new(0, 5).with_source([span(10, 12)]);
        let b = TextLocation::new(3, 9).with_source([span(2, 4), span(10, 12)]);
        let u = a.union(&b).expect("same page");
        // Non-contiguous source ranges kept distinct; the shared one deduped.
        assert_eq!(u.source, vec![span(2, 4), span(10, 12)]);
    }

    #[test]
    fn hash_covers_the_source_ranges() {
        let bare = TextLocation::new(0, 5);
        let with = TextLocation::new(0, 5).with_source([span(2, 4)]);
        // Same decoded span, different source pointer → different hash, so
        // tampering with the source is detectable.
        assert_ne!(bare.hash(), with.hash());
    }

    #[test]
    fn hash_is_stable_regardless_of_source_order() {
        let a = TextLocation::new(0, 5).with_source([span(2, 4), span(10, 12)]);
        let b = TextLocation::new(0, 5).with_source([span(10, 12), span(2, 4)]);
        // Normalized, so accumulation order does not change the hash.
        assert_eq!(a.hash(), b.hash());
    }
}
