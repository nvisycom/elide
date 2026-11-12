//! [`TabularLocation`]: a cell-addressed location within tabular content.

use std::cmp::Ordering;
use std::ops::Range;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::{ModalityLocation, Overlap};

/// A cell in tabular content by row and column, optionally a byte range in it.
///
/// Identifies a cell by zero-based [`row_index`] and
/// [`column_index`], optionally narrowed to a byte
/// range within the cell text for an entity that spans only part of the
/// cell. [`sheet_name`] scopes the cell to one sheet of
/// a multi-sheet workbook; [`column_name`] is a
/// human-readable header carried for provenance.
///
/// Overlap and ordering treat the sheet, cell, and byte range as
/// coordinates. The column name is redundant with the column index, so it
/// is carried but excluded from comparison.
///
/// [`row_index`]: Self::row_index
/// [`column_index`]: Self::column_index
/// [`sheet_name`]: Self::sheet_name
/// [`column_name`]: Self::column_name
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TabularLocation {
    /// Zero-based row index of the cell.
    pub row_index: u32,
    /// Zero-based column index of the cell.
    pub column_index: u32,
    /// The byte range of the cell text the entity spans. Unset means the
    /// whole cell.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub span: Option<Range<usize>>,
    /// Header label of the column, when known.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub column_name: Option<HipStr<'static>>,
    /// Sheet name within a multi-sheet workbook, when known.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub sheet_name: Option<HipStr<'static>>,
}

impl TabularLocation {
    /// Location addressing a whole cell at `(row_index, column_index)`,
    /// every optional field unset.
    pub fn new(row_index: u32, column_index: u32) -> Self {
        Self {
            row_index,
            column_index,
            span: None,
            column_name: None,
            sheet_name: None,
        }
    }

    /// Narrow the location to the byte range `span` of the cell text, for an
    /// entity that spans only part of the cell.
    #[must_use]
    pub fn with_span(mut self, span: Range<usize>) -> Self {
        self.span = Some(span);
        self
    }

    /// Attach the column's header label.
    #[must_use]
    pub fn with_column_name(mut self, column_name: impl Into<HipStr<'static>>) -> Self {
        self.column_name = Some(column_name.into());
        self
    }

    /// Attach the sheet name.
    #[must_use]
    pub fn with_sheet_name(mut self, sheet_name: impl Into<HipStr<'static>>) -> Self {
        self.sheet_name = Some(sheet_name.into());
        self
    }

    /// Whether two locations address the same sheet and cell.
    fn same_cell(&self, other: &Self) -> bool {
        self.sheet_name == other.sheet_name
            && self.row_index == other.row_index
            && self.column_index == other.column_index
    }
}

impl ModalityLocation for TabularLocation {
    fn overlap(&self, other: &Self) -> Overlap {
        // Different sheet or cell never overlaps, even at equal offsets.
        if !self.same_cell(other) {
            return Overlap::Disjoint;
        }
        // A missing span means "the whole cell". Two sub-cell spans compare
        // by their bytes; anything involving a whole cell is handled by the
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
            // `self` is the whole cell: it contains the other (a sub-range,
            // or, reflexively, another whole cell).
            (None, _) => Overlap::Contains,
            // `self` is a sub-range and `other` is the whole cell.
            (Some(_), None) => Overlap::ContainedBy,
        }
    }

    fn union(&self, other: &Self) -> Option<Self> {
        // A single redactable span can't cross cells; require the same one.
        if !self.same_cell(other) {
            return None;
        }
        // Union the intra-cell spans; a whole-cell side (no span) absorbs the
        // other, so the union is the whole cell.
        let mut location = Self::new(self.row_index, self.column_index);
        if let (Some(a), Some(b)) = (&self.span, &other.span) {
            location = location.with_span(a.start.min(b.start)..a.end.max(b.end));
        }
        // Carry the cell's descriptive labels from `self`.
        if let Some(name) = &self.column_name {
            location = location.with_column_name(name.clone());
        }
        if let Some(sheet) = &self.sheet_name {
            location = location.with_sheet_name(sheet.clone());
        }
        Some(location)
    }

    fn span_cmp(&self, other: &Self) -> Ordering {
        // By intra-cell extent: a whole-cell location is the most specific
        // (largest) and sorts above any sub-cell range.
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
        // Reading order: sheet, then row, then column, then intra-cell span.
        // An unnamed sheet and the whole cell sort first.
        let span = |location: &Self| location.span.as_ref().map(|span| (span.start, span.end));
        self.sheet_name
            .cmp(&other.sheet_name)
            .then(self.row_index.cmp(&other.row_index))
            .then(self.column_index.cmp(&other.column_index))
            .then(span(self).cmp(&span(other)))
    }

    fn hash(&self) -> Vec<u8> {
        fn span(bytes: &mut Vec<u8>, span: &Option<Range<usize>>) {
            match span {
                Some(span) => {
                    bytes.push(1);
                    bytes.extend_from_slice(&(span.start as u64).to_le_bytes());
                    bytes.extend_from_slice(&(span.end as u64).to_le_bytes());
                }
                None => bytes.push(0),
            }
        }
        fn name(bytes: &mut Vec<u8>, n: &Option<HipStr<'static>>) {
            match n {
                Some(value) => {
                    // Length-prefix the string so two adjacent names cannot be
                    // re-split into the same bytes (e.g. `("a", "bc")` vs
                    // `("a\x01b", "c")`); a bare presence byte is ambiguous when
                    // the value itself can contain that byte.
                    bytes.push(1);
                    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
                    bytes.extend_from_slice(value.as_bytes());
                }
                None => bytes.push(0),
            }
        }
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&self.row_index.to_le_bytes());
        bytes.extend_from_slice(&self.column_index.to_le_bytes());
        span(&mut bytes, &self.span);
        name(&mut bytes, &self.column_name);
        name(&mut bytes, &self.sheet_name);
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlaps_requires_same_sheet_and_cell() {
        let a = TabularLocation::new(0, 0);
        let b = TabularLocation::new(0, 0);
        assert!(a.overlaps(&b));
        // Different column never overlaps.
        assert!(!a.overlaps(&TabularLocation::new(0, 1)));
        // Different sheet never overlaps, same cell.
        let a_s1 = a.clone().with_sheet_name("Sheet1");
        let b_s2 = b.with_sheet_name("Sheet2");
        assert!(!a_s1.overlaps(&b_s2));
    }

    #[test]
    fn overlaps_intersects_byte_ranges_within_a_cell() {
        let a = TabularLocation::new(2, 3).with_span(0..5);
        let b = TabularLocation::new(2, 3).with_span(4..9);
        assert!(a.overlaps(&b));
        // Touching but disjoint ranges do not overlap.
        let c = TabularLocation::new(2, 3).with_span(5..9);
        assert!(!a.overlaps(&c));
        // A whole-cell location overlaps any sub-cell range.
        let whole = TabularLocation::new(2, 3);
        assert!(whole.overlaps(&c));
    }

    #[test]
    fn span_cmp_prefers_whole_cell_then_longer_range() {
        let whole = TabularLocation::new(0, 0);
        let part = TabularLocation::new(0, 0).with_span(0..4);
        assert_eq!(whole.span_cmp(&part), Ordering::Greater);
        let short = TabularLocation::new(0, 0).with_span(0..2);
        let long = TabularLocation::new(0, 0).with_span(0..8);
        assert_eq!(short.span_cmp(&long), Ordering::Less);
    }

    #[test]
    fn position_cmp_is_reading_order() {
        let r0c5 = TabularLocation::new(0, 5);
        let r1c0 = TabularLocation::new(1, 0);
        // Earlier row sorts first regardless of column.
        assert_eq!(r0c5.position_cmp(&r1c0), Ordering::Less);
        // Same cell: earlier offset first.
        let a = TabularLocation::new(0, 0).with_span(0..3);
        let b = TabularLocation::new(0, 0).with_span(4..7);
        assert_eq!(a.position_cmp(&b), Ordering::Less);
    }

    #[test]
    fn union_within_a_cell_bounds_the_ranges() {
        let a = TabularLocation::new(2, 3).with_span(0..5);
        let b = TabularLocation::new(2, 3).with_span(4..9);
        let u = a.union(&b).expect("same cell");
        assert_eq!(u.span, Some(0..9));
        // A whole-cell side absorbs the other into the whole cell.
        let whole = TabularLocation::new(2, 3);
        let u2 = whole.union(&a).expect("same cell");
        assert_eq!(u2.span, None);
    }

    #[test]
    fn union_across_cells_is_none() {
        let a = TabularLocation::new(0, 0).with_span(0..5);
        let b = TabularLocation::new(0, 1).with_span(0..5);
        // A single redactable span can't cross two cells.
        assert_eq!(a.union(&b), None);
    }

    #[test]
    fn column_name_does_not_affect_overlap() {
        let a = TabularLocation::new(0, 0).with_column_name("email");
        let b = TabularLocation::new(0, 0).with_column_name("e-mail");
        // Same cell, differing only by header label, still overlaps.
        assert!(a.overlaps(&b));
    }
}
