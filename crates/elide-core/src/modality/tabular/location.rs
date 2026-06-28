//! [`TabularLocation`]: a cell-addressed location within tabular content.

use std::cmp::Ordering;

use hipstr::HipStr;
#[cfg(feature = "schema")]
use schemars::JsonSchema;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::{ModalityLocation, Overlap};

/// Cell-addressed location within tabular content.
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
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct TabularLocation {
    /// Zero-based row index of the cell.
    pub row_index: u32,
    /// Zero-based column index of the cell.
    pub column_index: u32,
    /// Byte offset within the cell text where the entity starts. Unset
    /// means the whole cell.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub start_offset: Option<usize>,
    /// Byte offset within the cell text where the entity ends (exclusive).
    /// Unset means the whole cell.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub end_offset: Option<usize>,
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
            start_offset: None,
            end_offset: None,
            column_name: None,
            sheet_name: None,
        }
    }

    /// Narrow the location to the byte range `start..end` within the cell
    /// text, for an entity that spans only part of the cell.
    #[must_use]
    pub fn with_range(mut self, start: usize, end: usize) -> Self {
        self.start_offset = Some(start);
        self.end_offset = Some(end);
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

    /// Intra-cell byte length when a range is set, else `None` for a
    /// whole-cell location.
    fn range_len(&self) -> Option<usize> {
        match (self.start_offset, self.end_offset) {
            (Some(start), Some(end)) => Some(end.saturating_sub(start)),
            _ => None,
        }
    }
}

impl ModalityLocation for TabularLocation {
    fn overlap(&self, other: &Self) -> Overlap {
        // Different sheet or cell never overlaps, even at equal offsets.
        if !self.same_cell(other) {
            return Overlap::Disjoint;
        }
        // A missing offset means "the whole cell". Two sub-cell ranges
        // compare by their bytes; anything involving a whole cell is handled
        // by the containment rules below.
        match (
            self.start_offset,
            self.end_offset,
            other.start_offset,
            other.end_offset,
        ) {
            (Some(a_start), Some(a_end), Some(b_start), Some(b_end)) => {
                if a_start >= b_end || b_start >= a_end {
                    return Overlap::Disjoint;
                }
                if a_start <= b_start && b_end <= a_end {
                    return Overlap::Contains;
                }
                if b_start <= a_start && a_end <= b_end {
                    return Overlap::ContainedBy;
                }
                let inter = a_end.min(b_end) - a_start.max(b_start);
                let union = a_end.max(b_end) - a_start.min(b_start);
                Overlap::Crossing {
                    iou: inter as f32 / union as f32,
                }
            }
            // `self` is the whole cell: it contains the other (a sub-range,
            // or — reflexively — another whole cell).
            (None, _, _, _) | (_, None, _, _) => Overlap::Contains,
            // `self` is a sub-range and `other` is the whole cell.
            _ => Overlap::ContainedBy,
        }
    }

    fn union(&self, other: &Self) -> Option<Self> {
        // A single redactable span can't cross cells; require the same one.
        if !self.same_cell(other) {
            return None;
        }
        // Union the intra-cell ranges; a whole-cell side (unset offsets)
        // absorbs the other, so the union is the whole cell.
        let range = match (
            self.start_offset,
            self.end_offset,
            other.start_offset,
            other.end_offset,
        ) {
            (Some(a_start), Some(a_end), Some(b_start), Some(b_end)) => {
                Some((a_start.min(b_start), a_end.max(b_end)))
            }
            _ => None,
        };
        let mut location = Self::new(self.row_index, self.column_index);
        if let Some((start, end)) = range {
            location = location.with_range(start, end);
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
        match (self.range_len(), other.range_len()) {
            (Some(a), Some(b)) => a.cmp(&b),
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
        }
    }

    fn position_cmp(&self, other: &Self) -> Ordering {
        // Reading order: sheet, then row, then column, then intra-cell
        // offset. An unnamed sheet and an unset offset sort first.
        self.sheet_name
            .cmp(&other.sheet_name)
            .then(self.row_index.cmp(&other.row_index))
            .then(self.column_index.cmp(&other.column_index))
            .then(
                self.start_offset
                    .unwrap_or(0)
                    .cmp(&other.start_offset.unwrap_or(0)),
            )
            .then(
                self.end_offset
                    .unwrap_or(0)
                    .cmp(&other.end_offset.unwrap_or(0)),
            )
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
        let a = TabularLocation::new(2, 3).with_range(0, 5);
        let b = TabularLocation::new(2, 3).with_range(4, 9);
        assert!(a.overlaps(&b));
        // Touching but disjoint ranges do not overlap.
        let c = TabularLocation::new(2, 3).with_range(5, 9);
        assert!(!a.overlaps(&c));
        // A whole-cell location overlaps any sub-cell range.
        let whole = TabularLocation::new(2, 3);
        assert!(whole.overlaps(&c));
    }

    #[test]
    fn span_cmp_prefers_whole_cell_then_longer_range() {
        let whole = TabularLocation::new(0, 0);
        let part = TabularLocation::new(0, 0).with_range(0, 4);
        assert_eq!(whole.span_cmp(&part), Ordering::Greater);
        let short = TabularLocation::new(0, 0).with_range(0, 2);
        let long = TabularLocation::new(0, 0).with_range(0, 8);
        assert_eq!(short.span_cmp(&long), Ordering::Less);
    }

    #[test]
    fn position_cmp_is_reading_order() {
        let r0c5 = TabularLocation::new(0, 5);
        let r1c0 = TabularLocation::new(1, 0);
        // Earlier row sorts first regardless of column.
        assert_eq!(r0c5.position_cmp(&r1c0), Ordering::Less);
        // Same cell: earlier offset first.
        let a = TabularLocation::new(0, 0).with_range(0, 3);
        let b = TabularLocation::new(0, 0).with_range(4, 7);
        assert_eq!(a.position_cmp(&b), Ordering::Less);
    }

    #[test]
    fn union_within_a_cell_bounds_the_ranges() {
        let a = TabularLocation::new(2, 3).with_range(0, 5);
        let b = TabularLocation::new(2, 3).with_range(4, 9);
        let u = a.union(&b).expect("same cell");
        assert_eq!((u.start_offset, u.end_offset), (Some(0), Some(9)));
        // A whole-cell side absorbs the other into the whole cell.
        let whole = TabularLocation::new(2, 3);
        let u2 = whole.union(&a).expect("same cell");
        assert_eq!((u2.start_offset, u2.end_offset), (None, None));
    }

    #[test]
    fn union_across_cells_is_none() {
        let a = TabularLocation::new(0, 0).with_range(0, 5);
        let b = TabularLocation::new(0, 1).with_range(0, 5);
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
