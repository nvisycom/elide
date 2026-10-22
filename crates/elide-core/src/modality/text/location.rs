//! [`TextLocation`]: where an entity sits in text — a decoded span, plus the
//! anchors that say where that text physically lives.

use std::cmp::Ordering;
use std::ops::Range;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::SourceRef;
use crate::modality::{ModalityLocation, Overlap};
use crate::primitive::UnitBoundingBox;

/// Where an entity sits in text.
///
/// Two orthogonal facts:
///
/// - [`span`](Self::span): the byte range of the match in the decoded detection
///   text. Present for every entity a recognizer found over text; `None` only
///   for a selection with no decoded text (a reviewer marking the rendered
///   medium). This is the coordinate detection, reconciliation, and redaction
///   work in.
/// - [`anchors`](Self::anchors): where that text *physically* lives, so a
///   consumer can edit the original bytes or paint a highlight — zero or more
///   [`TextAnchor`]s. **Empty means the span is its own location** (plain text,
///   CSV: the decoded text *is* the source). A codec whose decoded text differs
///   from its raw bytes (OOXML, JSON escapes) attaches [`Bytes`](TextAnchor::Bytes)
///   anchors; a rendering codec (PDF) attaches [`Rendered`](TextAnchor::Rendered)
///   anchors, one per line the span wraps across.
///
/// Ordering, overlap, and union are defined on the `span`; the anchors ride
/// along (concatenated on union) but do not change comparison beyond equality.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TextLocation {
    /// The match's byte range in the decoded text, or `None` for a selection
    /// with no decoded text (a reviewer marking the rendered medium).
    #[cfg_attr(feature = "schema", schemars(with = "Option<Range<usize>>"))]
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub span: Option<Range<usize>>,
    /// Where the text physically lives. Empty when the span is its own source
    /// (plain text); otherwise raw-byte and/or rendered-page anchors.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub anchors: Vec<TextAnchor>,
}

/// Where a span's text physically lives: raw source bytes, or a rendered-page
/// region. A location may carry several (a span wrapping two lines of a PDF, or
/// drawn from several raw runs of an OOXML part).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum TextAnchor {
    /// Raw source bytes the span decoded from (accounting for escapes like
    /// `&amp;`), optionally within a named container part (a DOCX member). The
    /// bytes a redaction edits.
    Bytes(SourceRef),
    /// A region on a rendered page: a 1-based page number and a page-normalized
    /// box. What a viewer paints to highlight the entity.
    Rendered {
        /// 1-based page number.
        page: u32,
        /// The region in page-normalized `0.0..=1.0` coordinates (top-left).
        rect: UnitBoundingBox,
    },
}

impl TextAnchor {
    /// The raw-byte reference, if this is a [`Bytes`](Self::Bytes) anchor.
    fn as_bytes(&self) -> Option<&SourceRef> {
        match self {
            TextAnchor::Bytes(source) => Some(source),
            TextAnchor::Rendered { .. } => None,
        }
    }

    /// The page number, if this is a [`Rendered`](Self::Rendered) anchor.
    fn page(&self) -> Option<u32> {
        match self {
            TextAnchor::Rendered { page, .. } => Some(*page),
            TextAnchor::Bytes(_) => None,
        }
    }
}

impl TextLocation {
    /// A location over the decoded span `start..end`, with no anchors (the span
    /// is its own source — plain text). Attach anchors with
    /// [`with_source`](Self::with_source) / [`with_rects`](Self::with_rects).
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            span: Some(start..end),
            anchors: Vec::new(),
        }
    }

    /// A location with no decoded span, anchored only at raw source bytes (a
    /// reviewer marking the medium). Normalizes the refs (sorted, deduplicated).
    pub fn from_source(source: impl IntoIterator<Item = SourceRef>) -> Self {
        Self {
            span: None,
            anchors: bytes_anchors(source),
        }
    }

    /// Replace the raw-byte anchors with `source` (normalized), keeping the span
    /// and any rendered anchors. The raw bytes the decoded span came from.
    #[must_use]
    pub fn with_source(mut self, source: impl IntoIterator<Item = SourceRef>) -> Self {
        self.anchors.retain(|a| !matches!(a, TextAnchor::Bytes(_)));
        self.anchors.extend(bytes_anchors(source));
        self.anchors.sort_unstable_by(anchor_order);
        self
    }

    /// Replace the rendered-page anchors with one per `(page, rect)` in `rects`,
    /// keeping the span and any byte anchors. `None` clears them.
    #[must_use]
    pub fn with_rects(
        mut self,
        page: u32,
        rects: impl IntoIterator<Item = UnitBoundingBox>,
    ) -> Self {
        self.anchors
            .retain(|a| !matches!(a, TextAnchor::Rendered { .. }));
        self.anchors.extend(
            rects
                .into_iter()
                .map(|rect| TextAnchor::Rendered { page, rect }),
        );
        self.anchors.sort_unstable_by(anchor_order);
        self
    }

    /// The decoded byte range, or `None` for a selection with no decoded text.
    #[must_use]
    pub fn range(&self) -> Option<&Range<usize>> {
        self.span.as_ref()
    }

    /// The raw-byte source references this span decoded from (possibly empty).
    #[must_use]
    pub fn source(&self) -> Vec<SourceRef> {
        self.anchors
            .iter()
            .filter_map(TextAnchor::as_bytes)
            .cloned()
            .collect()
    }

    /// The rendered-page rectangles for the span, one per line it wraps across
    /// (empty when the text has no rendered form).
    #[must_use]
    pub fn rects(&self) -> Vec<UnitBoundingBox> {
        self.anchors
            .iter()
            .filter_map(|a| match a {
                TextAnchor::Rendered { rect, .. } => Some(*rect),
                TextAnchor::Bytes(_) => None,
            })
            .collect()
    }

    /// The 1-based page the span is rendered on (the lowest, for a span that
    /// wraps a page break), or `None` when it has no rendered anchor.
    #[must_use]
    pub fn page(&self) -> Option<u32> {
        self.anchors.iter().filter_map(TextAnchor::page).min()
    }

    /// The effective page for ordering/hashing: the rendered page, or 0 when the
    /// location has no rendered anchor (so unpaged text sorts before page 1).
    fn effective_page(&self) -> u32 {
        self.page().unwrap_or(0)
    }

    /// Whether this is a source-only selection (no decoded span).
    #[must_use]
    pub const fn is_source_only(&self) -> bool {
        self.span.is_none()
    }

    /// Byte length of the decoded span, or `None` when there is none.
    #[must_use]
    pub fn len(&self) -> Option<usize> {
        self.span.as_ref().map(|s| s.end.saturating_sub(s.start))
    }

    /// Whether the decoded span is empty (zero length). A source-only location
    /// has no span, so this is `false` for it.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == Some(0)
    }
}

/// Build normalized [`Bytes`](TextAnchor::Bytes) anchors from source refs.
fn bytes_anchors(source: impl IntoIterator<Item = SourceRef>) -> Vec<TextAnchor> {
    let mut refs: Vec<SourceRef> = source.into_iter().collect();
    SourceRef::normalize(&mut refs);
    refs.into_iter().map(TextAnchor::Bytes).collect()
}

/// Canonical order of anchors within a location, so equal locations compare and
/// hash identically: all byte anchors (by their `SourceRef` order) before all
/// rendered anchors (by page then rect).
fn anchor_order(a: &TextAnchor, b: &TextAnchor) -> Ordering {
    match (a, b) {
        (TextAnchor::Bytes(x), TextAnchor::Bytes(y)) => x.cmp(y),
        (TextAnchor::Bytes(_), TextAnchor::Rendered { .. }) => Ordering::Less,
        (TextAnchor::Rendered { .. }, TextAnchor::Bytes(_)) => Ordering::Greater,
        (TextAnchor::Rendered { page: p, rect: r }, TextAnchor::Rendered { page: q, rect: s }) => {
            p.cmp(q).then_with(|| rect_order(r, s))
        }
    }
}

/// A total order on normalized boxes (their fields are floats, so ordered by
/// bit patterns — only used to canonicalize anchor order, never geometrically).
fn rect_order(a: &UnitBoundingBox, b: &UnitBoundingBox) -> Ordering {
    let key = |r: &UnitBoundingBox| {
        [
            r.x.to_bits(),
            r.y.to_bits(),
            r.width.to_bits(),
            r.height.to_bits(),
        ]
    };
    key(a).cmp(&key(b))
}

impl ModalityLocation for TextLocation {
    fn overlap(&self, other: &Self) -> Overlap {
        // Range geometry is defined on the decoded span. A source-only selection
        // has no span and never range-dedups, so any pair involving one is
        // Disjoint.
        let (Some(a), Some(b)) = (&self.span, &other.span) else {
            return Overlap::Disjoint;
        };
        span_overlap(a, b)
    }

    fn union(&self, other: &Self) -> Option<Self> {
        // A single span can't straddle two pages; require page agreement (both
        // unpaged, or both on the same rendered page).
        if self.effective_page() != other.effective_page() {
            return None;
        }
        // Fusion unions decoded spans only; a source-only selection has no span
        // to enclose, so a pair involving one has no union.
        let (Some(a), Some(b)) = (&self.span, &other.span) else {
            return None;
        };
        // The fused entity spans both detections, so it carries both sides'
        // anchors (raw runs and rendered lines), canonicalized and deduped.
        let mut anchors = self.anchors.clone();
        anchors.extend(other.anchors.iter().cloned());
        anchors.sort_unstable_by(anchor_order);
        anchors.dedup();
        Some(Self {
            span: Some(a.start.min(b.start)..a.end.max(b.end)),
            anchors,
        })
    }

    fn span_cmp(&self, other: &Self) -> Ordering {
        // Extent: the decoded span's length; a source-only selection has none and
        // sorts as zero-length.
        self.len().unwrap_or(0).cmp(&other.len().unwrap_or(0))
    }

    fn position_cmp(&self, other: &Self) -> Ordering {
        // Reading order: rendered page first (unpaged = page 0), then the span.
        // A span-bearing location sorts before a source-only one (which has no
        // position); two source-only sort by their anchors.
        self.effective_page()
            .cmp(&other.effective_page())
            .then_with(|| span_cmp_for_order(&self.span, &other.span))
            .then_with(|| self.anchors.cmp_by(&other.anchors))
    }

    fn hash(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        // Effective page, then the span, then the anchors — the tamper-evidence
        // fold. (The box floats in rendered anchors are folded by bit pattern.)
        bytes.extend_from_slice(&self.effective_page().to_le_bytes());
        match &self.span {
            Some(span) => {
                bytes.push(1);
                bytes.extend_from_slice(&(span.start as u64).to_le_bytes());
                bytes.extend_from_slice(&(span.end as u64).to_le_bytes());
            }
            None => bytes.push(0),
        }
        hash_anchors(&mut bytes, &self.anchors);
        bytes
    }
}

/// Reading-order comparison of two optional spans: a present span sorts before
/// an absent one (a source-only selection has no position); two present spans by
/// start then end; two absent are equal (the anchors break the tie).
fn span_cmp_for_order(a: &Option<Range<usize>>, b: &Option<Range<usize>>) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => x.start.cmp(&y.start).then(x.end.cmp(&y.end)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// How two spans relate, by range IoU.
fn span_overlap(a: &Range<usize>, b: &Range<usize>) -> Overlap {
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

/// Fold the anchor list into the tamper-evidence hash: the count, then each
/// anchor by kind (byte refs by range+part, rendered by page+rect bits).
fn hash_anchors(bytes: &mut Vec<u8>, anchors: &[TextAnchor]) {
    bytes.extend_from_slice(&(anchors.len() as u64).to_le_bytes());
    for anchor in anchors {
        match anchor {
            TextAnchor::Bytes(source) => {
                bytes.push(0);
                bytes.extend_from_slice(&(source.range.start as u64).to_le_bytes());
                bytes.extend_from_slice(&(source.range.end as u64).to_le_bytes());
                match &source.part {
                    Some(part) => {
                        bytes.push(1);
                        bytes.extend_from_slice(&(part.len() as u64).to_le_bytes());
                        bytes.extend_from_slice(part.as_bytes());
                    }
                    None => bytes.push(0),
                }
            }
            TextAnchor::Rendered { page, rect } => {
                bytes.push(1);
                bytes.extend_from_slice(&page.to_le_bytes());
                for bits in [
                    rect.x.to_bits(),
                    rect.y.to_bits(),
                    rect.width.to_bits(),
                    rect.height.to_bits(),
                ] {
                    bytes.extend_from_slice(&bits.to_le_bytes());
                }
            }
        }
    }
}

/// Element-wise comparison of two anchor lists for a total order, so
/// `position_cmp` agrees with `Eq` (two locations with the same page and span
/// but different anchors are distinct and must not compare Equal).
trait AnchorsOrd {
    fn cmp_by(&self, other: &Self) -> Ordering;
}

impl AnchorsOrd for Vec<TextAnchor> {
    fn cmp_by(&self, other: &Self) -> Ordering {
        for (a, b) in self.iter().zip(other.iter()) {
            let ord = anchor_order(a, b);
            if ord != Ordering::Equal {
                return ord;
            }
        }
        self.len().cmp(&other.len())
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::*;

    fn span(start: usize, end: usize) -> SourceRef {
        SourceRef::new(start..end)
    }

    #[test]
    fn position_cmp_orders_by_start_then_end() {
        let a = TextLocation::new(0, 5);
        let b = TextLocation::new(3, 8);
        let c = TextLocation::new(3, 4);
        assert_eq!(a.position_cmp(&b), Ordering::Less);
        assert_eq!(c.position_cmp(&b), Ordering::Less);
        assert_eq!(b.position_cmp(&a), Ordering::Greater);
    }

    #[test]
    fn position_cmp_orders_pages_before_offsets() {
        let early_page =
            TextLocation::new(100, 110).with_rects(1, [UnitBoundingBox::new(0.0, 0.0, 0.1, 0.1)]);
        let late_page =
            TextLocation::new(0, 5).with_rects(2, [UnitBoundingBox::new(0.0, 0.0, 0.1, 0.1)]);
        // Page 1 sorts before page 2 even with a larger offset.
        assert_eq!(early_page.position_cmp(&late_page), Ordering::Less);
    }

    #[test]
    fn overlap_classifies_the_relationship() {
        let a = TextLocation::new(0, 10);
        assert_eq!(a.overlap(&TextLocation::new(10, 20)), Overlap::Disjoint);
        assert_eq!(a.overlap(&TextLocation::new(2, 8)), Overlap::Contains);
        assert_eq!(TextLocation::new(2, 8).overlap(&a), Overlap::ContainedBy);
        assert_eq!(a.overlap(&a), Overlap::Contains);
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
        assert_eq!(a.union(&a), Some(a.clone()));
    }

    #[test]
    fn union_requires_same_page() {
        let a = TextLocation::new(0, 5).with_rects(1, [UnitBoundingBox::new(0.0, 0.0, 0.1, 0.1)]);
        let b = TextLocation::new(3, 12).with_rects(2, [UnitBoundingBox::new(0.0, 0.0, 0.1, 0.1)]);
        assert_eq!(a.union(&b), None);
    }

    #[test]
    fn span_cmp_is_extent_not_position() {
        let short_late = TextLocation::new(10, 12);
        let long_early = TextLocation::new(0, 9);
        assert_eq!(long_early.position_cmp(&short_late), Ordering::Less);
        assert_eq!(long_early.span_cmp(&short_late), Ordering::Greater);
    }

    #[test]
    fn with_source_normalizes_sorted_and_deduped() {
        let loc = TextLocation::new(0, 10).with_source([span(5, 8), span(1, 3), span(5, 8)]);
        assert_eq!(loc.source(), vec![span(1, 3), span(5, 8)]);
    }

    #[test]
    fn union_concatenates_and_normalizes_source() {
        let a = TextLocation::new(0, 5).with_source([span(10, 12)]);
        let b = TextLocation::new(3, 9).with_source([span(2, 4), span(10, 12)]);
        let u = a.union(&b).expect("same page");
        assert_eq!(u.source(), vec![span(2, 4), span(10, 12)]);
    }

    #[test]
    fn hash_covers_the_source_ranges() {
        let bare = TextLocation::new(0, 5);
        let with = TextLocation::new(0, 5).with_source([span(2, 4)]);
        assert_ne!(bare.hash(), with.hash());
    }

    #[test]
    fn hash_is_stable_regardless_of_source_order() {
        let a = TextLocation::new(0, 5).with_source([span(2, 4), span(10, 12)]);
        let b = TextLocation::new(0, 5).with_source([span(10, 12), span(2, 4)]);
        assert_eq!(a.hash(), b.hash());
    }

    #[test]
    fn source_only_has_no_range_and_does_not_overlap() {
        let src = TextLocation::from_source([span(3, 12)]);
        assert!(src.is_source_only());
        assert_eq!(src.range(), None);
        assert_eq!(src.len(), None);
        assert_eq!(src.source(), vec![span(3, 12)]);
        assert_eq!(src.overlap(&TextLocation::new(3, 12)), Overlap::Disjoint);
        assert_eq!(src.overlap(&src), Overlap::Disjoint);
        assert_eq!(src.union(&TextLocation::new(3, 12)), None);
    }

    #[test]
    fn source_only_sorts_after_decoded_and_hashes_distinctly() {
        let decoded = TextLocation::new(100, 110);
        let src = TextLocation::from_source([span(0, 5)]);
        assert_eq!(decoded.position_cmp(&src), Ordering::Less);
        assert_eq!(src.position_cmp(&decoded), Ordering::Greater);
        assert_ne!(TextLocation::new(0, 5).hash(), src.hash());
    }

    #[test]
    fn ordering_agrees_with_eq_on_anchors() {
        // Same span, different byte anchors: distinct under Eq, so position_cmp
        // must not call them Equal (a BTreeSet/binary_search would drop one).
        let a = TextLocation::new(0, 5).with_source([span(1, 2)]);
        let b = TextLocation::new(0, 5).with_source([span(9, 10)]);
        assert_ne!(a, b);
        assert_ne!(a.position_cmp(&b), Ordering::Equal);
    }

    #[test]
    fn rendered_anchors_expose_page_and_rects() {
        let r1 = UnitBoundingBox::new(0.1, 0.2, 0.3, 0.05);
        let r2 = UnitBoundingBox::new(0.1, 0.3, 0.2, 0.05);
        let loc = TextLocation::new(0, 20).with_rects(2, [r1, r2]);
        assert_eq!(loc.page(), Some(2));
        assert_eq!(loc.rects(), vec![r1, r2]);
        // Byte and rendered anchors coexist on one location.
        let both = loc.with_source([span(100, 120)]);
        assert_eq!(both.page(), Some(2));
        assert_eq!(both.rects(), vec![r1, r2]);
        assert_eq!(both.source(), vec![span(100, 120)]);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serializes_and_round_trips() {
        let src = TextLocation::from_source([span(3, 12)]);
        let json = serde_json::to_string(&src).expect("serializes");
        let back: TextLocation = serde_json::from_str(&json).expect("round-trips");
        assert_eq!(back, src);
        assert!(back.is_source_only());

        let decoded = TextLocation::new(0, 5).with_source([span(2, 4)]);
        let json = serde_json::to_string(&decoded).expect("serializes");
        let back: TextLocation = serde_json::from_str(&json).expect("round-trips");
        assert_eq!(back, decoded);

        let rendered =
            TextLocation::new(0, 5).with_rects(3, [UnitBoundingBox::new(0.1, 0.2, 0.3, 0.05)]);
        let json = serde_json::to_string(&rendered).expect("serializes");
        let back: TextLocation = serde_json::from_str(&json).expect("round-trips");
        assert_eq!(back, rendered);
    }
}
