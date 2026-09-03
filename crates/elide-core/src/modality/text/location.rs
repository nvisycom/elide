//! [`TextLocation`]: where an entity sits in text, a decoded byte range or a
//! source-only reference.

use std::cmp::Ordering;
use std::ops::Range;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::SourceRef;
use crate::modality::{ModalityLocation, Overlap};

/// Where an entity sits in text: a [coordinate](TextCoord) (a decoded byte range,
/// or a source-only reference) plus an optional page number.
///
/// The coordinate is either a [`Decoded`](TextCoord::Decoded) byte range in the
/// pipeline's text stream, or a [`Source`](TextCoord::Source)-only reference for
/// content with no decoded range (a reviewer selecting rendered text). The page
/// is orthogonal to the coordinate kind, so it sits alongside rather than inside.
///
/// Ordering and overlap consider only the coordinate; the page is carried for
/// codecs that page their text but does not affect comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TextLocation {
    /// The coordinate: a decoded range, or a source-only reference.
    pub coord: TextCoord,
    /// 1-based page number, when known. Orthogonal to the coordinate kind.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub page: Option<u32>,
}

/// The coordinate of a [`TextLocation`]: a decoded byte range, or a source-only
/// reference.
///
/// The distinction is whether a decoded range exists at all. A recognizer over
/// decoded text produces a [`Decoded`](Self::Decoded) span (with the raw source
/// it decodes from, when the codec's decoded text differs from the source). A
/// reviewer marking rendered text has no decoded range, only where the selection
/// sits in the raw bytes, a [`Source`](Self::Source).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum TextCoord {
    /// A decoded byte range in the pipeline's text stream, with the raw
    /// [`SourceRef`]s it came from when the codec's decoded text differs from the
    /// source (empty for plain text / CSV, where decoded == source).
    Decoded(DecodedSpan),
    /// Source-only: no decoded range (a reviewer selecting rendered text), only
    /// where it sits in the raw bytes. Non-empty by construction.
    Source(Vec<SourceRef>),
}

/// A decoded byte range and the raw source it decodes from.
///
/// The `range` is a half-open `[start, end)` span in the decoded text stream.
/// `source` carries the exact raw byte range(s) that span came from for codecs
/// whose decoded text differs from the source (XML/HTML/DOCX entity decoding,
/// JSON escapes); it is empty when the source equals the decoded text.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DecodedSpan {
    /// Byte range within the (decoded) text content.
    #[cfg_attr(feature = "schema", schemars(with = "Range<usize>"))]
    pub range: Range<usize>,
    /// The exact raw source ranges this decoded span came from. Empty when the
    /// source equals the decoded text.
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

impl DecodedSpan {
    /// A decoded span over `range`, with no source refs.
    pub fn new(range: Range<usize>) -> Self {
        Self {
            range,
            source: Vec::new(),
        }
    }

    /// Attach the exact raw source references, normalizing them (sorted,
    /// deduplicated).
    #[must_use]
    pub fn with_source(mut self, source: impl IntoIterator<Item = SourceRef>) -> Self {
        self.source = source.into_iter().collect();
        SourceRef::normalize(&mut self.source);
        self
    }

    /// Byte length of the decoded range.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.range.end.saturating_sub(self.range.start)
    }

    /// Whether the decoded range is empty (zero length).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How this decoded span sits against `other`, by range. Total and
    /// unconditional: decoded geometry never sees a missing range.
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

    /// The smallest decoded span covering both, with their source refs
    /// concatenated and normalized (kept distinct, merged runs and escape-split
    /// spans are genuinely non-contiguous in the raw bytes).
    fn union(&self, other: &Self) -> Self {
        let mut source = self.source.clone();
        source.extend_from_slice(&other.source);
        SourceRef::normalize(&mut source);
        Self {
            range: self.range.start.min(other.range.start)..self.range.end.max(other.range.end),
            source,
        }
    }
}

impl TextLocation {
    /// A decoded location covering `start..end`, page unset and no source refs.
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            coord: TextCoord::Decoded(DecodedSpan::new(start..end)),
            page: None,
        }
    }

    /// A source-only location, for content with no decoded range (a reviewer
    /// selecting rendered text). Normalizes the refs (sorted, deduplicated).
    pub fn from_source(source: impl IntoIterator<Item = SourceRef>) -> Self {
        let mut source: Vec<SourceRef> = source.into_iter().collect();
        SourceRef::normalize(&mut source);
        Self {
            coord: TextCoord::Source(source),
            page: None,
        }
    }

    /// Set the 1-based page number, consuming and returning `self`.
    #[must_use]
    pub fn with_page(mut self, page: Option<u32>) -> Self {
        self.page = page;
        self
    }

    /// Attach the exact raw source references to a *decoded* location,
    /// normalizing them. A no-op on a source-only location (its refs are its
    /// whole coordinate, set at construction).
    #[must_use]
    pub fn with_source(mut self, source: impl IntoIterator<Item = SourceRef>) -> Self {
        if let TextCoord::Decoded(span) = &mut self.coord {
            span.source = source.into_iter().collect();
            SourceRef::normalize(&mut span.source);
        }
        self
    }

    /// The decoded byte range, or `None` for a source-only location.
    #[must_use]
    pub fn range(&self) -> Option<&Range<usize>> {
        match &self.coord {
            TextCoord::Decoded(span) => Some(&span.range),
            TextCoord::Source(_) => None,
        }
    }

    /// The raw source references: a decoded span's (possibly empty), or the
    /// source-only refs.
    #[must_use]
    pub fn source(&self) -> &[SourceRef] {
        match &self.coord {
            TextCoord::Decoded(span) => &span.source,
            TextCoord::Source(refs) => refs,
        }
    }

    /// The 1-based page number, when known.
    #[must_use]
    pub const fn page(&self) -> Option<u32> {
        self.page
    }

    /// Whether this is a source-only location (no decoded range).
    #[must_use]
    pub const fn is_source_only(&self) -> bool {
        matches!(self.coord, TextCoord::Source(_))
    }

    /// The decoded span, if this is a decoded location.
    #[must_use]
    pub fn decoded(&self) -> Option<&DecodedSpan> {
        match &self.coord {
            TextCoord::Decoded(span) => Some(span),
            TextCoord::Source(_) => None,
        }
    }

    /// Byte length of the decoded range, or `None` for a source-only location
    /// (which has no length).
    #[must_use]
    pub fn len(&self) -> Option<usize> {
        self.decoded().map(DecodedSpan::len)
    }

    /// Whether the decoded range is empty. A source-only location is not empty in
    /// this sense (it has no range), so this returns `false` for it.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.decoded().is_some_and(DecodedSpan::is_empty)
    }
}

impl ModalityLocation for TextLocation {
    fn overlap(&self, other: &Self) -> Overlap {
        // Range geometry is defined on decoded coordinates. A source-only
        // location has no decoded range and is injected after detection's
        // reconcile pass, so it never range-dedups: any pair involving one is
        // Disjoint, not comparable in a shared coordinate space.
        match (&self.coord, &other.coord) {
            (TextCoord::Decoded(a), TextCoord::Decoded(b)) => a.overlap(b),
            _ => Overlap::Disjoint,
        }
    }

    fn union(&self, other: &Self) -> Option<Self> {
        // A single byte range can't span two pages; require agreement.
        if self.page != other.page {
            return None;
        }
        // Fusion only ever unions decoded detections. A source-only coordinate
        // is not fused (no range to enclose), so a pair involving one has no
        // union.
        match (&self.coord, &other.coord) {
            (TextCoord::Decoded(a), TextCoord::Decoded(b)) => Some(Self {
                coord: TextCoord::Decoded(a.union(b)),
                page: self.page,
            }),
            _ => None,
        }
    }

    fn span_cmp(&self, other: &Self) -> Ordering {
        // Extent: a decoded span's length; a source-only location has no extent
        // and sorts as zero-length.
        let a = self.len().unwrap_or(0);
        let b = other.len().unwrap_or(0);
        a.cmp(&b)
    }

    fn position_cmp(&self, other: &Self) -> Ordering {
        // Reading order: page first (unpaged sorts as page 0), then the
        // coordinate. Decoded coordinates order by start then end; a source-only
        // coordinate has no decoded position, so all decoded sort before all
        // source-only, and two source-only order by their first source ref.
        self.page
            .unwrap_or(0)
            .cmp(&other.page.unwrap_or(0))
            .then_with(|| self.coord.cmp(&other.coord))
    }

    fn hash(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        match self.page {
            Some(page) => {
                bytes.push(1);
                bytes.extend_from_slice(&page.to_le_bytes());
            }
            None => bytes.push(0),
        }
        match &self.coord {
            TextCoord::Decoded(span) => {
                bytes.push(0); // coordinate tag: decoded
                bytes.extend_from_slice(&(span.range.start as u64).to_le_bytes());
                bytes.extend_from_slice(&(span.range.end as u64).to_le_bytes());
                hash_source(&mut bytes, &span.source);
            }
            TextCoord::Source(refs) => {
                bytes.push(1); // coordinate tag: source-only
                hash_source(&mut bytes, refs);
            }
        }
        bytes
    }
}

/// Reading order over coordinates: all [`Decoded`](TextCoord::Decoded) sort
/// before all [`Source`](TextCoord::Source) (a source-only coordinate has no
/// decoded position); decoded coordinates order by range start then end; two
/// source-only coordinates order by their source refs ([`SourceRef`] is itself
/// [`Ord`], so the lists compare element-wise).
impl Ord for TextCoord {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (TextCoord::Decoded(x), TextCoord::Decoded(y)) => x
                .range
                .start
                .cmp(&y.range.start)
                .then(x.range.end.cmp(&y.range.end)),
            (TextCoord::Decoded(_), TextCoord::Source(_)) => Ordering::Less,
            (TextCoord::Source(_), TextCoord::Decoded(_)) => Ordering::Greater,
            (TextCoord::Source(x), TextCoord::Source(y)) => x.cmp(y),
        }
    }
}

impl PartialOrd for TextCoord {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Fold a normalized source-ref list into the tamper-evidence hash: the count,
/// then each ref's range and optional part.
fn hash_source(bytes: &mut Vec<u8>, source: &[SourceRef]) {
    bytes.extend_from_slice(&(source.len() as u64).to_le_bytes());
    for src in source {
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
        assert_eq!(u.range(), Some(&(0..12)));
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
        assert_eq!(loc.source(), &[span(1, 3), span(5, 8)]);
    }

    #[test]
    fn union_concatenates_and_normalizes_source() {
        let a = TextLocation::new(0, 5).with_source([span(10, 12)]);
        let b = TextLocation::new(3, 9).with_source([span(2, 4), span(10, 12)]);
        let u = a.union(&b).expect("same page");
        // Non-contiguous source ranges kept distinct; the shared one deduped.
        assert_eq!(u.source(), &[span(2, 4), span(10, 12)]);
    }

    #[test]
    fn hash_covers_the_source_ranges() {
        let bare = TextLocation::new(0, 5);
        let with = TextLocation::new(0, 5).with_source([span(2, 4)]);
        // Same decoded span, different source pointer -> different hash, so
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

    #[test]
    fn source_only_has_no_range_and_does_not_overlap() {
        let src = TextLocation::from_source([span(3, 12)]);
        assert!(src.is_source_only());
        assert_eq!(src.range(), None);
        assert_eq!(src.len(), None);
        assert_eq!(src.source(), &[span(3, 12)]);
        // A source-only location never range-dedups: Disjoint against anything.
        assert_eq!(src.overlap(&TextLocation::new(3, 12)), Overlap::Disjoint);
        assert_eq!(src.overlap(&src), Overlap::Disjoint);
        // And has no union with a decoded location.
        assert_eq!(src.union(&TextLocation::new(3, 12)), None);
    }

    #[test]
    fn source_only_sorts_after_decoded_and_hashes_distinctly() {
        let decoded = TextLocation::new(100, 110);
        let src = TextLocation::from_source([span(0, 5)]);
        // All decoded sort before all source-only, regardless of offset.
        assert_eq!(decoded.position_cmp(&src), Ordering::Less);
        assert_eq!(src.position_cmp(&decoded), Ordering::Greater);
        // A decoded and a source-only location with the "same" bytes hash
        // differently (distinct coordinate kinds).
        assert_ne!(TextLocation::new(0, 5).hash(), src.hash());
    }
}
