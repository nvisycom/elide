//! [`TextReplacement`]: what a text operator produces.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::ModalityReplacement;

/// What a text operator produces: a substitution or a removal.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum TextReplacement {
    /// Replace the span with this value.
    Substituted(String),
    /// Remove the span entirely.
    Removed,
}

impl TextReplacement {
    /// Substitution with the given value.
    pub fn substituted(value: impl Into<String>) -> Self {
        Self::Substituted(value.into())
    }

    /// Replacement value, or `None` for a removal.
    pub fn value(&self) -> Option<&str> {
        match self {
            Self::Substituted(value) => Some(value),
            Self::Removed => None,
        }
    }
}

impl ModalityReplacement for TextReplacement {}
