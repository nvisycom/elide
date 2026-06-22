//! [`TabularReplacement`]: what a tabular operator produces.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::ModalityReplacement;
use crate::modality::text::TextReplacement;

/// What a tabular operator produces.
///
/// A cell holds text, so most treatments are a [`TextReplacement`] applied
/// to the cell, carried as [`Cell`]. Structural treatments that the text
/// model can't express — dropping a whole [`row`] or [`column`] — are their
/// own variants.
///
/// [`Cell`]: TabularReplacement::Cell
/// [`row`]: TabularReplacement::DropRow
/// [`column`]: TabularReplacement::DropColumn
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
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

impl ModalityReplacement for TabularReplacement {}
