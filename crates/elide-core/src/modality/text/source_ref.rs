//! [`SourceRef`]: a pointer from a detected span back to the original source.

use std::cmp::Ordering;
use std::ops::Range;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A reference back to the original source: a byte range, and, for a container
/// whose body spans several files, which part that range indexes.
///
/// [`TextLocation`]'s `range` indexes the *decoded* text stream a codec hands
/// the pipeline (entities resolved, container parts concatenated). A `SourceRef`
/// is the *exact raw* byte range the decoded range came from, accounting for XML
/// escapes where `&amp;` (5 raw bytes) decodes to `&` (1). `part` names the
/// container file the range is in (`word/header1.xml`) for a multi-file body
/// like DOCX, and is `None` for a single-file source (XML, HTML). It lets a
/// consumer point back at the untouched source bytes.
///
/// [`TextLocation`]: super::TextLocation
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SourceRef {
    /// The raw source byte range.
    #[cfg_attr(feature = "schema", schemars(with = "Range<usize>"))]
    pub range: Range<usize>,
    /// The container part the range indexes, for a multi-file body; `None` for a
    /// single-file source.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub part: Option<HipStr<'static>>,
}

impl SourceRef {
    /// A source reference to `range` in a single-file source (no part).
    pub fn new(range: Range<usize>) -> Self {
        Self { range, part: None }
    }

    /// A source reference to `range` within the named container `part`.
    pub fn in_part(range: Range<usize>, part: impl Into<HipStr<'static>>) -> Self {
        Self {
            range,
            part: Some(part.into()),
        }
    }

    /// Put a collection of source refs into canonical order and drop exact
    /// duplicates, leaving non-contiguous ranges distinct, so equal source sets
    /// compare and hash identically regardless of how they were accumulated.
    pub(super) fn normalize(refs: &mut Vec<SourceRef>) {
        refs.sort_unstable();
        refs.dedup();
    }
}

/// Canonical total order: by part, then range start, then range end.
/// `Range<usize>` is not `Ord`, so this is defined by hand rather than derived.
/// It is the single source of truth `normalize` orders and dedups by, and that
/// [`TextLocation`](super::TextLocation)'s hash folds in order.
impl Ord for SourceRef {
    fn cmp(&self, other: &Self) -> Ordering {
        self.part
            .cmp(&other.part)
            .then(self.range.start.cmp(&other.range.start))
            .then(self.range.end.cmp(&other.range.end))
    }
}

impl PartialOrd for SourceRef {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(start: usize, end: usize) -> SourceRef {
        SourceRef::new(start..end)
    }

    #[test]
    fn normalize_sorts_and_drops_exact_duplicates() {
        let mut refs = vec![r(5, 8), r(1, 3), r(5, 8)];
        SourceRef::normalize(&mut refs);
        assert_eq!(refs, vec![r(1, 3), r(5, 8)]);
    }

    #[test]
    fn normalize_keeps_same_range_in_different_parts_distinct() {
        // Same byte range, different container files: not duplicates.
        let mut refs = vec![
            SourceRef::in_part(0..4, "word/header1.xml"),
            SourceRef::in_part(0..4, "word/document.xml"),
            SourceRef::in_part(0..4, "word/document.xml"),
        ];
        SourceRef::normalize(&mut refs);
        // Deduped to two, ordered by part name.
        assert_eq!(
            refs,
            vec![
                SourceRef::in_part(0..4, "word/document.xml"),
                SourceRef::in_part(0..4, "word/header1.xml"),
            ]
        );
    }

    #[test]
    fn normalize_is_order_independent() {
        let mut a = vec![r(10, 12), r(2, 4)];
        let mut b = vec![r(2, 4), r(10, 12)];
        SourceRef::normalize(&mut a);
        SourceRef::normalize(&mut b);
        assert_eq!(a, b);
    }
}
