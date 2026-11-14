//! [`TabularReplacement`]: what a tabular operator produces.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::ModalityReplacement;
use crate::modality::text::TextReplacement;

/// What a tabular operator produces: a cell treatment, or a dropped row or
/// column.
///
/// A cell holds text, so most treatments are a [`TextReplacement`] applied
/// to the cell, carried as [`Cell`]. Structural treatments that the text
/// model can't express, dropping a whole [`row`] or [`column`], are their
/// own variants.
///
/// [`Cell`]: TabularReplacement::Cell
/// [`row`]: TabularReplacement::DropRow
/// [`column`]: TabularReplacement::DropColumn
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum TabularReplacement {
    /// Apply a text treatment to the cell.
    Cell(TextReplacement),
    /// Drop the entire row the entity sits in.
    DropRow,
    /// Drop the entire column the entity sits in.
    DropColumn,
}

impl From<TextReplacement> for TabularReplacement {
    fn from(replacement: TextReplacement) -> Self {
        Self::Cell(replacement)
    }
}

/// The cell treatment, or the replacement back when it drops a row or column.
impl TryFrom<TabularReplacement> for TextReplacement {
    type Error = TabularReplacement;

    fn try_from(replacement: TabularReplacement) -> Result<Self, Self::Error> {
        match replacement {
            TabularReplacement::Cell(text) => Ok(text),
            structural => Err(structural),
        }
    }
}

impl ModalityReplacement for TabularReplacement {
    fn removed() -> Self {
        Self::Cell(TextReplacement::Removed)
    }
}

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
            TextReplacement::try_from(TabularReplacement::DropRow),
            Err(TabularReplacement::DropRow)
        );
    }
}
