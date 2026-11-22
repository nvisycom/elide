//! [`TabularReplacement`]: what a tabular operator produces.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::ModalityReplacement;
use crate::modality::text::TextReplacement;

/// What a tabular operator produces: a cell treatment, or a dropped row or
/// column.
///
/// A cell holds text, so most treatments are a [`TextReplacement`] applied
/// to the cell, carried as [`CellRedacted`]. Structural treatments that the
/// text model can't express, dropping a whole [`row`] or [`column`], are
/// their own variants.
///
/// [`CellRedacted`]: TabularReplacement::CellRedacted
/// [`row`]: TabularReplacement::RowDropped
/// [`column`]: TabularReplacement::ColumnDropped
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum TabularReplacement {
    /// The cell's text is redacted by this treatment.
    CellRedacted(TextReplacement),
    /// The entire row the entity sits in is dropped.
    RowDropped,
    /// The entire column the entity sits in is dropped.
    ColumnDropped,
}

impl From<TextReplacement> for TabularReplacement {
    fn from(replacement: TextReplacement) -> Self {
        Self::CellRedacted(replacement)
    }
}

/// The cell treatment, or the replacement back when it drops a row or column.
impl TryFrom<TabularReplacement> for TextReplacement {
    type Error = TabularReplacement;

    fn try_from(replacement: TabularReplacement) -> Result<Self, Self::Error> {
        match replacement {
            TabularReplacement::CellRedacted(text) => Ok(text),
            structural => Err(structural),
        }
    }
}

impl ModalityReplacement for TabularReplacement {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cell_treatment_converts_back_to_text() {
        let cell = TabularReplacement::from(TextReplacement::Removed);
        assert_eq!(
            TextReplacement::try_from(cell),
            Ok(TextReplacement::Removed)
        );
        assert_eq!(
            TextReplacement::try_from(TabularReplacement::RowDropped),
            Err(TabularReplacement::RowDropped)
        );
    }
}
