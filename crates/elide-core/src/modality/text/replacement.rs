//! [`TextReplacement`]: what a text operator produces.

use std::borrow::Cow;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::ModalityReplacement;

/// What a text operator produces: a removal, a cover, or a replacement.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum TextReplacement {
    /// The span is cut out and the text closes up: nothing marks where it
    /// was.
    Removed,
    /// The span is hidden in place: its extent is kept and visibly marked.
    /// Each format draws the mark: a box over it in a PDF, one `█` per
    /// character in text.
    Covered,
    /// The span is replaced by this text.
    Replaced(String),
}

impl TextReplacement {
    /// The mark a covered span is written as in text, one per character: a
    /// full block, `█`. A format that can't hold it draws its own.
    pub const MARK: char = '\u{2588}';

    /// A replacement by `value`.
    pub fn replaced(value: impl Into<String>) -> Self {
        Self::Replaced(value.into())
    }

    /// The text written in place of `original`, the span it hides: nothing
    /// for a removal, one `█` per character for a cover, the replacement
    /// text for a replacement.
    pub fn written(&self, original: &str) -> Cow<'_, str> {
        match self {
            Self::Removed => Cow::Borrowed(""),
            Self::Covered => Cow::Owned(original.chars().map(|_| Self::MARK).collect()),
            Self::Replaced(value) => Cow::Borrowed(value),
        }
    }
}

impl ModalityReplacement for TextReplacement {
    fn removed() -> Self {
        Self::Removed
    }

    fn covered() -> Self {
        Self::Covered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_replacement_writes_its_text() {
        assert_eq!(TextReplacement::Removed.written("Jane Roe"), "");
        assert_eq!(TextReplacement::Covered.written("Jané Roe"), "████████");
        assert_eq!(
            TextReplacement::replaced("[NAME]").written("Jane Roe"),
            "[NAME]"
        );
    }
}
