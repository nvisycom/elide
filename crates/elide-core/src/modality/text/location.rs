//! [`TextLocation`]: where an entity sits in text — a [`Detected`] span the
//! pipeline found over decoded text, or a reviewer's [`Marked`] selection on a
//! medium with no decoded text. Each carries where that text physically lives.

use std::cmp::Ordering;
use std::ops::Range;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::SourceRef;
use crate::modality::{ModalityLocation, Overlap};
use crate::primitive::UnitBoundingBox;

/// Where an entity sits in text.
///
/// Two independent facts, one per level:
///
/// - **Is there a decoded span?** A [`Detected`] location was found by a
///   recognizer over the decoded detection text and always carries a byte span;
///   a [`Marked`] location is a reviewer's selection on a medium with no decoded
///   text (a PDF region, or a specific container part) and has none.
/// - **Where does the text physically live?** Each level's variant says: the span
///   is its own source (plain text, [`Detected::Inline`]), raw bytes elsewhere
///   (OOXML/JSON escapes, the `Source` variants), or a rendered-page region (PDF,
///   the `Rendered` variants).
///
/// A reviewer marking plain text gives a byte range, so it is a
/// [`Detected::Inline`], not a `Marked` — which is why [`Marked`] has no inline
/// variant, and a location naming nothing is unrepresentable.
///
/// Ordering, overlap, and union are defined on the decoded span (a [`Marked`]
/// location has none, so it never range-dedups); the physical anchor rides along.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum TextLocation {
    /// The pipeline found this over decoded text; it always has a span.
    Detected(Detected),
    /// A reviewer placed this with no decoded text; the anchor is the location.
    Marked(Marked),
}

/// Validating `Deserialize`: a location is read through the derived shape, then
/// checked. The derive alone would accept an empty collection straight off the
/// wire — a mark, or a source/rendered detection, that names nothing — but the
/// constructors keep every `Source` anchor non-empty and every `Rendered`
/// non-empty. Enforce the same invariant here so untrusted input cannot smuggle
/// in the degenerate state the enum exists to forbid, and normalize the refs so
/// a deserialized location hashes and compares like a constructed one.
#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for TextLocation {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Detected(Detected),
            Marked(Marked),
        }

        const EMPTY: &str = "a source anchor needs a ref and a rendered anchor needs a rect";
        let mut loc = match Repr::deserialize(deserializer)? {
            Repr::Detected(d) => TextLocation::Detected(d),
            Repr::Marked(m) => TextLocation::Marked(m),
        };
        match &mut loc {
            TextLocation::Detected(Detected::Inline { .. }) => {}
            TextLocation::Detected(Detected::Source { refs, .. })
            | TextLocation::Marked(Marked::Source { refs }) => {
                if refs.is_empty() {
                    return Err(D::Error::custom(EMPTY));
                }
                SourceRef::normalize(refs);
            }
            TextLocation::Detected(Detected::Rendered { rects, .. })
            | TextLocation::Marked(Marked::Rendered { rects, .. }) => {
                if rects.is_empty() {
                    return Err(D::Error::custom(EMPTY));
                }
            }
        }
        Ok(loc)
    }
}

/// A location the pipeline detected over decoded text: always a byte span, plus
/// where that text physically lives.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Detected {
    /// Plain text, CSV: the decoded span is its own source, no separate anchor.
    Inline {
        /// The match's byte range in the decoded text.
        #[cfg_attr(feature = "schema", schemars(with = "Range<usize>"))]
        span: Range<usize>,
    },
    /// OOXML, JSON escapes: the decoded span's raw source bytes differ from the
    /// decoded text. `refs` is non-empty.
    Source {
        /// The match's byte range in the decoded text.
        #[cfg_attr(feature = "schema", schemars(with = "Range<usize>"))]
        span: Range<usize>,
        /// The raw-byte references, normalized (sorted, deduplicated). Non-empty.
        #[cfg_attr(feature = "schema", schemars(length(min = 1)))]
        refs: Vec<SourceRef>,
    },
    /// PDF: the span has no flat byte coordinate, only a rendered-page region.
    /// `rects` is non-empty.
    Rendered {
        /// The match's byte range in the decoded (page-local) text.
        #[cfg_attr(feature = "schema", schemars(with = "Range<usize>"))]
        span: Range<usize>,
        /// 1-based page number.
        page: u32,
        /// The regions in page-normalized `0.0..=1.0` coordinates (top-left), one
        /// per line the span wraps across. Non-empty.
        #[cfg_attr(feature = "schema", schemars(length(min = 1)))]
        rects: Vec<UnitBoundingBox>,
    },
}

/// A reviewer's mark with no decoded span: the physical anchor is the whole
/// location. (A mark on plain text gives a byte range, so it is a
/// [`Detected::Inline`], not a `Marked` — hence no inline variant here.)
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Marked {
    /// Anchored at raw source bytes (a specific container part). `refs` non-empty.
    Source {
        /// The raw-byte references, normalized. Non-empty.
        #[cfg_attr(feature = "schema", schemars(length(min = 1)))]
        refs: Vec<SourceRef>,
    },
    /// Anchored at a rendered-page region. `rects` non-empty.
    Rendered {
        /// 1-based page number.
        page: u32,
        /// The regions in page-normalized coordinates. Non-empty.
        #[cfg_attr(feature = "schema", schemars(length(min = 1)))]
        rects: Vec<UnitBoundingBox>,
    },
}

impl TextLocation {
    /// A location over the decoded `span` whose text is its own source (plain
    /// text, CSV): no separate anchor.
    pub fn inline(span: Range<usize>) -> Self {
        TextLocation::Detected(Detected::Inline { span })
    }

    /// A detected location over `span`, anchored at raw source bytes (OOXML, JSON
    /// escapes). Empty `refs` means the span is its own source, so it falls back
    /// to [`inline`](Self::inline).
    pub fn source(span: Range<usize>, refs: impl IntoIterator<Item = SourceRef>) -> Self {
        match normalize_refs(refs) {
            Some(refs) => TextLocation::Detected(Detected::Source { span, refs }),
            None => TextLocation::inline(span),
        }
    }

    /// A detected location over `span`, anchored at rendered-page regions (PDF).
    /// Empty `rects` falls back to [`inline`](Self::inline).
    pub fn rendered(
        span: Range<usize>,
        page: u32,
        rects: impl IntoIterator<Item = UnitBoundingBox>,
    ) -> Self {
        let rects: Vec<_> = rects.into_iter().collect();
        if rects.is_empty() {
            TextLocation::inline(span)
        } else {
            TextLocation::Detected(Detected::Rendered { span, page, rects })
        }
    }

    /// A reviewer's mark with no decoded span, anchored at a single raw source
    /// reference. Taking one `SourceRef` (not a collection) keeps the anchor
    /// non-empty by construction — an empty mark is unspellable at the call site,
    /// not guarded at runtime. For a mark drawn from several refs (one entity
    /// aggregating non-adjacent raw runs), use [`marked_sources`](Self::marked_sources).
    pub fn marked_source(source: SourceRef) -> Self {
        TextLocation::Marked(Marked::Source { refs: vec![source] })
    }

    /// A reviewer's mark anchored at several raw source references (normalized),
    /// for an entity aggregating non-adjacent raw runs. Returns `None` when `refs`
    /// is empty — a mark must name a location, so the caller handles the empty
    /// case rather than producing one that redacts nothing.
    pub fn marked_sources(refs: impl IntoIterator<Item = SourceRef>) -> Option<Self> {
        normalize_refs(refs).map(|refs| TextLocation::Marked(Marked::Source { refs }))
    }

    /// A reviewer's mark with no decoded span, anchored at a single rendered-page
    /// region. One `rect` (not a collection) keeps the anchor non-empty by
    /// construction.
    pub fn marked_rendered(page: u32, rect: UnitBoundingBox) -> Self {
        TextLocation::Marked(Marked::Rendered {
            page,
            rects: vec![rect],
        })
    }

    /// Attach (or replace) a detected span's raw-byte anchor with `refs`
    /// (normalized), keeping the span. Empty `refs` leaves the span its own
    /// source (inline). A reviewer mark has no span to re-anchor and is returned
    /// unchanged.
    #[must_use]
    pub fn with_source(self, refs: impl IntoIterator<Item = SourceRef>) -> Self {
        match self.range() {
            Some(span) => Self::source(span.clone(), refs),
            None => self,
        }
    }

    /// Attach (or replace) a detected span's rendered anchor with one page's
    /// `rects`, keeping the span. Empty `rects` leaves the span its own source
    /// (inline). A reviewer mark has no span to re-anchor and is returned
    /// unchanged.
    #[must_use]
    pub fn with_rects(self, page: u32, rects: impl IntoIterator<Item = UnitBoundingBox>) -> Self {
        match self.range() {
            Some(span) => Self::rendered(span.clone(), page, rects),
            None => self,
        }
    }

    /// The decoded byte range, or `None` for a reviewer mark (which has none).
    #[must_use]
    pub fn range(&self) -> Option<&Range<usize>> {
        match self {
            TextLocation::Detected(Detected::Inline { span })
            | TextLocation::Detected(Detected::Source { span, .. })
            | TextLocation::Detected(Detected::Rendered { span, .. }) => Some(span),
            TextLocation::Marked(_) => None,
        }
    }

    /// The raw-byte source references for this location (empty when it has no
    /// raw-byte anchor).
    #[must_use]
    pub fn source_refs(&self) -> &[SourceRef] {
        match self {
            TextLocation::Detected(Detected::Source { refs, .. })
            | TextLocation::Marked(Marked::Source { refs }) => refs,
            _ => &[],
        }
    }

    /// The rendered-page rectangles for the span, one per line it wraps across
    /// (empty when it has no rendered anchor).
    #[must_use]
    pub fn rects(&self) -> &[UnitBoundingBox] {
        match self {
            TextLocation::Detected(Detected::Rendered { rects, .. })
            | TextLocation::Marked(Marked::Rendered { rects, .. }) => rects,
            _ => &[],
        }
    }

    /// The 1-based page the span is rendered on, or `None` when it has no rendered
    /// anchor.
    #[must_use]
    pub fn page(&self) -> Option<u32> {
        match self {
            TextLocation::Detected(Detected::Rendered { page, .. })
            | TextLocation::Marked(Marked::Rendered { page, .. }) => Some(*page),
            _ => None,
        }
    }

    /// The effective page for ordering/hashing: the rendered page, or 0 when the
    /// location has no rendered anchor (so unpaged text sorts before page 1).
    fn effective_page(&self) -> u32 {
        self.page().unwrap_or(0)
    }

    /// Whether this is a reviewer's mark (no decoded span).
    #[must_use]
    pub const fn is_marked(&self) -> bool {
        matches!(self, TextLocation::Marked(_))
    }

    /// Byte length of the decoded span, or `None` for a reviewer mark.
    #[must_use]
    pub fn len(&self) -> Option<usize> {
        self.range().map(|s| s.end.saturating_sub(s.start))
    }

    /// Whether the decoded span is empty (zero length). A reviewer mark has no
    /// span, so this is `false` for it.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == Some(0)
    }

    /// Translate the decoded span forward by `delta` bytes, keeping the physical
    /// anchor, or `None` for a reviewer mark (which has no span to move).
    ///
    /// This is the coordinate lift when the enclosing text's bytes *are* the
    /// local text shifted by a fixed offset — a recognizer's finding over a
    /// sub-window rebased onto the window's start in the full text. It moves only
    /// the decoded `span`; the raw-byte `refs` and rendered `rects` are physical
    /// anchors that a decoded-offset shift does not relocate, so the caller uses
    /// this only where the local finding is [`Inline`](Detected::Inline) (its
    /// span is its own source), and the rebuilt location stays inline.
    #[must_use]
    pub fn offset(&self, delta: usize) -> Option<Self> {
        let span = self.range()?;
        Some(TextLocation::inline(span.start + delta..span.end + delta))
    }
}

/// Collect source refs into canonical order (sorted, deduplicated), or `None`
/// when empty — an empty byte anchor has no location.
fn normalize_refs(refs: impl IntoIterator<Item = SourceRef>) -> Option<Vec<SourceRef>> {
    let mut refs: Vec<SourceRef> = refs.into_iter().collect();
    if refs.is_empty() {
        return None;
    }
    SourceRef::normalize(&mut refs);
    Some(refs)
}

impl ModalityLocation for TextLocation {
    fn overlap(&self, other: &Self) -> Overlap {
        // A span is page-local, so two spans only share a coordinate space when
        // they are on the same rendered page (both unpaged, or equal page). Spans
        // on different pages are disjoint even with identical byte ranges.
        if self.effective_page() != other.effective_page() {
            return Overlap::Disjoint;
        }
        // Range geometry is defined on the decoded span. A reviewer mark has no
        // span and never range-dedups, so any pair involving one is Disjoint.
        let (Some(a), Some(b)) = (self.range(), other.range()) else {
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
        // Fusion unions decoded spans only; a reviewer mark has no span to
        // enclose, so a pair involving one has no union.
        let (Some(a), Some(b)) = (self.range(), other.range()) else {
            return None;
        };
        let span = a.start.min(b.start)..a.end.max(b.end);
        // The fused entity spans both detections, so it carries both sides' raw
        // runs and rendered rects, merged on the shared page.
        let refs = merge_refs(self.source_refs(), other.source_refs());
        let rects = merge_rects(self, other);
        Some(match (refs, rects) {
            (_, Some((page, rects))) => {
                TextLocation::Detected(Detected::Rendered { span, page, rects })
            }
            (Some(refs), None) => TextLocation::Detected(Detected::Source { span, refs }),
            (None, None) => TextLocation::Detected(Detected::Inline { span }),
        })
    }

    fn span_cmp(&self, other: &Self) -> Ordering {
        // Extent: the decoded span's length; a reviewer mark has none and sorts as
        // zero-length.
        self.len().unwrap_or(0).cmp(&other.len().unwrap_or(0))
    }

    fn position_cmp(&self, other: &Self) -> Ordering {
        // Reading order: rendered page first (unpaged = page 0), then the span. A
        // span-bearing location sorts before a reviewer mark (which has no
        // position); two marks sort by their anchor payloads.
        self.effective_page()
            .cmp(&other.effective_page())
            .then_with(|| span_cmp_for_order(self.range(), other.range()))
            .then_with(|| self.source_refs().cmp(other.source_refs()))
            .then_with(|| rects_order(self.rects(), other.rects()))
    }

    fn hash(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        // Effective page, then the span, then the physical anchor — the
        // tamper-evidence fold. (Box floats are folded by bit pattern.)
        bytes.extend_from_slice(&self.effective_page().to_le_bytes());
        match self.range() {
            Some(span) => {
                bytes.push(1);
                bytes.extend_from_slice(&(span.start as u64).to_le_bytes());
                bytes.extend_from_slice(&(span.end as u64).to_le_bytes());
            }
            None => bytes.push(0),
        }
        hash_refs(&mut bytes, self.source_refs());
        hash_rects(&mut bytes, self.rects());
        bytes
    }
}

/// Merge two spans' raw-byte refs (normalized), or `None` when both are empty.
fn merge_refs(a: &[SourceRef], b: &[SourceRef]) -> Option<Vec<SourceRef>> {
    if a.is_empty() && b.is_empty() {
        return None;
    }
    let mut refs: Vec<SourceRef> = a.iter().chain(b).cloned().collect();
    SourceRef::normalize(&mut refs);
    Some(refs)
}

/// Merge two locations' rendered rects on their shared page, or `None` when
/// neither is rendered. (Union already required equal `effective_page`, so any
/// rendered anchors agree on the page.)
fn merge_rects(a: &TextLocation, b: &TextLocation) -> Option<(u32, Vec<UnitBoundingBox>)> {
    let page = a.page().or_else(|| b.page())?;
    let mut rects: Vec<UnitBoundingBox> = a.rects().iter().chain(b.rects()).copied().collect();
    rects.dedup();
    Some((page, rects))
}

/// Reading-order comparison of two optional spans: a present span sorts before an
/// absent one (a reviewer mark has no position); two present by start then end;
/// two absent are equal.
fn span_cmp_for_order(a: Option<&Range<usize>>, b: Option<&Range<usize>>) -> Ordering {
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

/// A total order on two rect lists, element-wise then by length.
fn rects_order(a: &[UnitBoundingBox], b: &[UnitBoundingBox]) -> Ordering {
    for (x, y) in a.iter().zip(b.iter()) {
        let ord = rect_order(x, y);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    a.len().cmp(&b.len())
}

/// A total order on normalized boxes (their fields are floats, so ordered by bit
/// patterns — only used to canonicalize order, never geometrically).
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

/// Fold raw-byte refs into the tamper-evidence hash: the count, then each ref by
/// range and optional part.
fn hash_refs(bytes: &mut Vec<u8>, refs: &[SourceRef]) {
    bytes.extend_from_slice(&(refs.len() as u64).to_le_bytes());
    for source in refs {
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
}

/// Fold rendered rects into the tamper-evidence hash: the count, then each box by
/// bit pattern.
fn hash_rects(bytes: &mut Vec<u8>, rects: &[UnitBoundingBox]) {
    bytes.extend_from_slice(&(rects.len() as u64).to_le_bytes());
    for rect in rects {
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

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::*;

    fn r(start: usize, end: usize) -> SourceRef {
        SourceRef::new(start..end)
    }

    #[test]
    fn position_cmp_orders_by_start_then_end() {
        let a = TextLocation::inline(0..5);
        let b = TextLocation::inline(3..8);
        let c = TextLocation::inline(3..4);
        assert_eq!(a.position_cmp(&b), Ordering::Less);
        assert_eq!(c.position_cmp(&b), Ordering::Less);
        assert_eq!(b.position_cmp(&a), Ordering::Greater);
    }

    #[test]
    fn position_cmp_orders_pages_before_offsets() {
        let early_page =
            TextLocation::rendered(100..110, 1, [UnitBoundingBox::new(0.0, 0.0, 0.1, 0.1)]);
        let late_page = TextLocation::rendered(0..5, 2, [UnitBoundingBox::new(0.0, 0.0, 0.1, 0.1)]);
        // Page 1 sorts before page 2 even with a larger offset.
        assert_eq!(early_page.position_cmp(&late_page), Ordering::Less);
    }

    #[test]
    fn overlap_classifies_the_relationship() {
        let a = TextLocation::inline(0..10);
        assert_eq!(a.overlap(&TextLocation::inline(10..20)), Overlap::Disjoint);
        assert_eq!(a.overlap(&TextLocation::inline(2..8)), Overlap::Contains);
        assert_eq!(TextLocation::inline(2..8).overlap(&a), Overlap::ContainedBy);
        assert_eq!(a.overlap(&a), Overlap::Contains);
        let Overlap::Crossing { iou } = a.overlap(&TextLocation::inline(5..15)) else {
            panic!("expected crossing");
        };
        assert!((iou - 5.0 / 15.0).abs() < 1e-6);
    }

    #[test]
    fn union_is_the_bounding_range() {
        let a = TextLocation::inline(0..5);
        let b = TextLocation::inline(3..12);
        let u = a.union(&b).expect("same page");
        assert_eq!(u.range(), Some(&(0..12)));
        assert_eq!(a.union(&a), Some(a.clone()));
    }

    #[test]
    fn union_requires_same_page() {
        let a = TextLocation::rendered(0..5, 1, [UnitBoundingBox::new(0.0, 0.0, 0.1, 0.1)]);
        let b = TextLocation::rendered(3..12, 2, [UnitBoundingBox::new(0.0, 0.0, 0.1, 0.1)]);
        assert_eq!(a.union(&b), None);
    }

    #[test]
    fn span_cmp_is_extent_not_position() {
        let short_late = TextLocation::inline(10..12);
        let long_early = TextLocation::inline(0..9);
        assert_eq!(long_early.position_cmp(&short_late), Ordering::Less);
        assert_eq!(long_early.span_cmp(&short_late), Ordering::Greater);
    }

    #[test]
    fn same_span_on_different_pages_is_disjoint() {
        // Spans are page-local, so identical byte ranges on different rendered
        // pages must not overlap (nor union) — else reconcile would wrongly merge
        // an entity on page 2 with an unrelated one at the same offset on page 3.
        let full = UnitBoundingBox::new(0.0, 0.0, 1.0, 1.0);
        let p2 = TextLocation::rendered(181..202, 2, [full]);
        let p3 = TextLocation::rendered(181..202, 3, [full]);
        assert_eq!(p2.overlap(&p3), Overlap::Disjoint);
        assert_eq!(p2.union(&p3), None);
        // Same page, same span: still overlaps as before.
        let p2b = TextLocation::rendered(181..202, 2, [full]);
        assert_eq!(p2.overlap(&p2b), Overlap::Contains);
    }

    #[test]
    fn with_source_normalizes_sorted_and_deduped() {
        let loc = TextLocation::inline(0..10).with_source([r(5, 8), r(1, 3), r(5, 8)]);
        assert_eq!(loc.source_refs(), &[r(1, 3), r(5, 8)]);
    }

    #[test]
    fn union_concatenates_and_normalizes_source() {
        let a = TextLocation::source(0..5, [r(10, 12)]);
        let b = TextLocation::source(3..9, [r(2, 4), r(10, 12)]);
        let u = a.union(&b).expect("same page");
        assert_eq!(u.source_refs(), &[r(2, 4), r(10, 12)]);
    }

    #[test]
    fn hash_covers_the_source_ranges() {
        let bare = TextLocation::inline(0..5);
        let with = TextLocation::source(0..5, [r(2, 4)]);
        assert_ne!(bare.hash(), with.hash());
    }

    #[test]
    fn hash_is_stable_regardless_of_source_order() {
        let a = TextLocation::source(0..5, [r(2, 4), r(10, 12)]);
        let b = TextLocation::source(0..5, [r(10, 12), r(2, 4)]);
        assert_eq!(a.hash(), b.hash());
    }

    #[test]
    fn marked_has_no_range_and_does_not_overlap() {
        let mark = TextLocation::marked_source(r(3, 12));
        assert!(mark.is_marked());
        assert_eq!(mark.range(), None);
        assert_eq!(mark.len(), None);
        assert_eq!(mark.source_refs(), &[r(3, 12)]);
        assert_eq!(
            mark.overlap(&TextLocation::inline(3..12)),
            Overlap::Disjoint
        );
        assert_eq!(mark.overlap(&mark), Overlap::Disjoint);
        assert_eq!(mark.union(&TextLocation::inline(3..12)), None);
    }

    #[test]
    fn marked_sorts_after_detected_and_hashes_distinctly() {
        let detected = TextLocation::inline(100..110);
        let mark = TextLocation::marked_source(r(0, 5));
        assert_eq!(detected.position_cmp(&mark), Ordering::Less);
        assert_eq!(mark.position_cmp(&detected), Ordering::Greater);
        assert_ne!(TextLocation::inline(0..5).hash(), mark.hash());
    }

    #[test]
    fn ordering_agrees_with_eq_on_anchors() {
        // Same span, different byte anchors: distinct under Eq, so position_cmp
        // must not call them Equal (a BTreeSet/binary_search would drop one).
        let a = TextLocation::source(0..5, [r(1, 2)]);
        let b = TextLocation::source(0..5, [r(9, 10)]);
        assert_ne!(a, b);
        assert_ne!(a.position_cmp(&b), Ordering::Equal);
    }

    #[test]
    fn rendered_exposes_page_and_rects() {
        let r1 = UnitBoundingBox::new(0.1, 0.2, 0.3, 0.05);
        let r2 = UnitBoundingBox::new(0.1, 0.3, 0.2, 0.05);
        let loc = TextLocation::rendered(0..20, 2, [r1, r2]);
        assert_eq!(loc.page(), Some(2));
        assert_eq!(loc.rects(), &[r1, r2]);
    }

    #[test]
    fn offset_translates_the_span_and_is_inline() {
        let loc = TextLocation::inline(3..8);
        let moved = loc.offset(100).expect("inline has a span");
        assert_eq!(moved.range(), Some(&(103..108)));
        assert!(!moved.is_marked());
    }

    #[test]
    fn offset_of_a_mark_is_none() {
        // A reviewer mark has no decoded span, so there is nothing to translate.
        let mark = TextLocation::marked_source(r(3, 12));
        assert_eq!(mark.offset(10), None);
    }

    #[test]
    fn inline_has_no_anchor() {
        let loc = TextLocation::inline(0..10);
        assert_eq!(loc.source_refs(), &[] as &[SourceRef]);
        assert_eq!(loc.rects(), &[] as &[UnitBoundingBox]);
        assert_eq!(loc.page(), None);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serializes_and_round_trips() {
        let cases = [
            TextLocation::inline(0..5),
            TextLocation::source(0..5, [r(2, 4)]),
            TextLocation::rendered(0..5, 3, [UnitBoundingBox::new(0.1, 0.2, 0.3, 0.05)]),
            TextLocation::marked_source(r(3, 12)),
            TextLocation::marked_rendered(2, UnitBoundingBox::new(0.0, 0.0, 1.0, 1.0)),
        ];
        for case in cases {
            let json = serde_json::to_string(&case).expect("serializes");
            let back: TextLocation = serde_json::from_str(&json).expect("round-trips");
            assert_eq!(back, case, "round-trip mismatch for {json}");
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn marked_source_round_trips_without_span() {
        // A marked `source` has no span and must deserialize as Marked, not as a
        // span-less Detected::Source (the untagged disambiguation).
        let mark = TextLocation::marked_source(r(3, 12));
        let json = serde_json::to_string(&mark).expect("serializes");
        assert!(
            !json.contains("span"),
            "marked source must omit span: {json}"
        );
        let back: TextLocation = serde_json::from_str(&json).expect("round-trips");
        assert!(back.is_marked());
        assert_eq!(back, mark);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn empty_object_is_rejected() {
        // The whole point of the enum: a location naming nothing is unspellable.
        assert!(serde_json::from_str::<TextLocation>("{}").is_err());
    }
}
