//! [`MetadataReplacement`]: what a metadata operator produces.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::ModalityReplacement;
use crate::modality::text::TextReplacement;

/// What a metadata operator produces: a text treatment, or the field as it is.
///
/// A field's value is text, so most treatments are a [`TextReplacement`]
/// applied to the field, or to the part of its value the location narrows to,
/// carried as [`Value`]: a removal of the whole field drops it. [`Unchanged`]
/// leaves the field as it is, which a substitution of its own text cannot do
/// for a field that holds no free text (a coordinate, a timestamp).
///
/// [`Value`]: MetadataReplacement::Value
/// [`Unchanged`]: MetadataReplacement::Unchanged
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum MetadataReplacement {
    /// Apply a text treatment to the field's value.
    Value(TextReplacement),
    /// Leave the field as it is.
    Unchanged,
}

impl From<TextReplacement> for MetadataReplacement {
    fn from(replacement: TextReplacement) -> Self {
        Self::Value(replacement)
    }
}

/// The value treatment, or the replacement back when it leaves the field as it
/// is.
impl TryFrom<MetadataReplacement> for TextReplacement {
    type Error = MetadataReplacement;

    fn try_from(replacement: MetadataReplacement) -> Result<Self, Self::Error> {
        match replacement {
            MetadataReplacement::Value(text) => Ok(text),
            unchanged @ MetadataReplacement::Unchanged => Err(unchanged),
        }
    }
}

impl ModalityReplacement for MetadataReplacement {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_treatment_converts_back_to_text() {
        let value = MetadataReplacement::from(TextReplacement::substituted("x"));
        assert_eq!(
            TextReplacement::try_from(value),
            Ok(TextReplacement::substituted("x"))
        );
        assert_eq!(
            TextReplacement::try_from(MetadataReplacement::Unchanged),
            Err(MetadataReplacement::Unchanged)
        );
    }
}
