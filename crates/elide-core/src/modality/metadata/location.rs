//! [`MetadataLocation`]: a metadata field addressed by key, optionally narrowed
//! to a byte range of its value.

use std::cmp::Ordering;
use std::ops::Range;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::{ModalityLocation, Overlap};

/// A metadata field, addressed by its key within a metadata modality, and
/// optionally narrowed to a byte range of its value.
///
/// The owning stream already says *which* scheme the field belongs to, so the
/// location only needs the field's [`key`](Self::key) within it
/// (`"GPSLatitude"`, `"modified"`). Without a [`span`](Self::span) it is the
/// whole field, what a format's label flags; with one it is that part of the
/// value, what a recognizer found inside a free-text field.
///
/// Overlap and ordering treat the key and the span as coordinates, the way a
/// tabular location treats its cell and span.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MetadataLocation {
    /// The field's identifier within its modality (e.g. `"GPSLatitude"`).
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub key: HipStr<'static>,
    /// The byte range of the value the entity spans. Unset means the whole
    /// field.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub span: Option<Range<usize>>,
}

impl MetadataLocation {
    /// Location addressing the whole field `key`.
    pub fn new(key: impl Into<HipStr<'static>>) -> Self {
        Self {
            key: key.into(),
            span: None,
        }
    }

    /// Narrow the location to the byte range `span` of the value, for an
    /// entity that spans only part of the field.
    #[must_use]
    pub fn with_span(mut self, span: Range<usize>) -> Self {
        self.span = Some(span);
        self
    }
}

impl ModalityLocation for MetadataLocation {
    fn overlap(&self, other: &Self) -> Overlap {
        // A different field never overlaps, even at equal offsets.
        if self.key != other.key {
            return Overlap::Disjoint;
        }
        // A missing span means "the whole field". Two spans compare by their
        // bytes; anything involving a whole field is handled by the
        // containment rules below.
        match (&self.span, &other.span) {
            (Some(a), Some(b)) => {
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
            // `self` is the whole field: it contains the other (a span, or,
            // reflexively, the whole field).
            (None, _) => Overlap::Contains,
            // `self` is a span and `other` is the whole field.
            (Some(_), None) => Overlap::ContainedBy,
        }
    }

    fn union(&self, other: &Self) -> Option<Self> {
        // A single redactable span can't cross fields; require the same one.
        if self.key != other.key {
            return None;
        }
        // Union the spans; a whole-field side (no span) absorbs the other, so
        // the union is the whole field.
        let mut location = Self::new(self.key.clone());
        if let (Some(a), Some(b)) = (&self.span, &other.span) {
            location = location.with_span(a.start.min(b.start)..a.end.max(b.end));
        }
        Some(location)
    }

    fn span_cmp(&self, other: &Self) -> Ordering {
        // By extent within the value: the whole field is the largest and
        // sorts above any span.
        match (
            self.span.as_ref().map(Range::len),
            other.span.as_ref().map(Range::len),
        ) {
            (Some(a), Some(b)) => a.cmp(&b),
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
        }
    }

    fn position_cmp(&self, other: &Self) -> Ordering {
        // Fields have no position in the document, so order by key, then by
        // span. The whole field sorts first.
        let span = |location: &Self| location.span.as_ref().map(|span| (span.start, span.end));
        self.key.cmp(&other.key).then(span(self).cmp(&span(other)))
    }

    fn hash(&self) -> Vec<u8> {
        // Length-prefix the key and mark the span's presence so the digest is
        // unambiguous.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(self.key.len() as u64).to_le_bytes());
        bytes.extend_from_slice(self.key.as_bytes());
        match &self.span {
            Some(span) => {
                bytes.push(1);
                bytes.extend_from_slice(&(span.start as u64).to_le_bytes());
                bytes.extend_from_slice(&(span.end as u64).to_le_bytes());
            }
            None => bytes.push(0),
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(key: &str, range: Range<usize>) -> MetadataLocation {
        MetadataLocation::new(key.to_owned()).with_span(range)
    }

    #[test]
    fn same_key_contains_itself_others_disjoint() {
        let a = MetadataLocation::new("GPSLatitude");
        assert!(matches!(a.overlap(&a), Overlap::Contains));
        assert!(a.overlaps(&a));

        let b = MetadataLocation::new("Make");
        assert!(matches!(a.overlap(&b), Overlap::Disjoint));
        assert!(!a.overlaps(&b));
    }

    #[test]
    fn the_whole_field_contains_any_part_of_it() {
        let whole = MetadataLocation::new("description");
        let part = part("description", 4..8);
        assert!(matches!(whole.overlap(&part), Overlap::Contains));
        assert!(matches!(part.overlap(&whole), Overlap::ContainedBy));
        assert!(matches!(part.overlap(&part.clone()), Overlap::Contains));
        assert!(matches!(
            part.overlap(&self::part("title", 4..8)),
            Overlap::Disjoint
        ));
    }

    #[test]
    fn parts_of_a_value_compare_by_their_bytes() {
        let a = part("description", 0..4);
        assert!(matches!(
            a.overlap(&part("description", 4..8)),
            Overlap::Disjoint
        ));
        assert!(matches!(
            a.overlap(&part("description", 1..3)),
            Overlap::Contains
        ));
        assert!(matches!(
            a.overlap(&part("description", 2..6)),
            Overlap::Crossing { .. }
        ));
    }

    #[test]
    fn union_only_within_the_same_field() {
        let a = MetadataLocation::new("GPSLatitude");
        assert_eq!(a.union(&a), Some(a.clone()));
        let b = MetadataLocation::new("Make");
        assert_eq!(a.union(&b), None);
        assert_eq!(
            part("description", 0..4).union(&part("description", 2..9)),
            Some(part("description", 0..9))
        );
        assert_eq!(
            part("description", 0..4).union(&MetadataLocation::new("description")),
            Some(MetadataLocation::new("description"))
        );
    }

    #[test]
    fn position_is_by_key_then_range() {
        let a = MetadataLocation::new("Make");
        let b = MetadataLocation::new("Model");
        assert_eq!(a.position_cmp(&b), Ordering::Less);
        assert_eq!(
            part("Make", 4..6).position_cmp(&part("Make", 0..2)),
            Ordering::Greater
        );
        assert_eq!(a.position_cmp(&part("Make", 0..2)), Ordering::Less);
    }

    #[test]
    fn hash_distinguishes_keys_and_ranges() {
        let a = MetadataLocation::new("Make");
        let b = MetadataLocation::new("Model");
        assert_ne!(a.hash(), b.hash());
        assert_ne!(a.hash(), part("Make", 0..2).hash());
        assert_ne!(part("Make", 0..2).hash(), part("Make", 0..3).hash());
    }
}
