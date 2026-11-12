//! [`TextValued`]: the modalities whose value is text, which the text
//! operators serve with one implementation each.

use elide_core::modality::Modality;
#[cfg(feature = "metadata")]
use elide_core::modality::metadata::{Metadata, MetadataData, MetadataReplacement};
#[cfg(feature = "tabular")]
use elide_core::modality::tabular::{Tabular, TabularReplacement};
use elide_core::modality::text::{Text, TextData, TextReplacement};

/// A modality whose value is text: plain text, a table cell, a metadata field.
///
/// The text operators ([`Mask`], [`Replace`], [`Sha2Hash`], …) read the value
/// as a string and produce a [`TextReplacement`]; this trait is how each
/// modality hands them its value and wraps their output. Sealed: implemented
/// for [`Text`], and for `Tabular` and `Metadata` behind their features.
///
/// [`Mask`]: crate::operators::Mask
/// [`Replace`]: crate::operators::Replace
/// [`Sha2Hash`]: crate::operators::Sha2Hash
pub trait TextValued: Modality + sealed::Sealed {
    /// The text of `data`.
    fn value(data: &Self::Data) -> &str;

    /// Wrap a text treatment as this modality's replacement.
    fn replacement(text: TextReplacement) -> Self::Replacement;

    /// The replacement that leaves `data` as it is.
    fn unchanged(data: &Self::Data) -> Self::Replacement;

    /// The text treatment inside `replacement`, if it carries one.
    fn text_of(replacement: &Self::Replacement) -> Option<&TextReplacement>;
}

impl TextValued for Text {
    fn value(data: &TextData) -> &str {
        data.as_str()
    }

    fn replacement(text: TextReplacement) -> TextReplacement {
        text
    }

    fn unchanged(data: &TextData) -> TextReplacement {
        TextReplacement::substituted(data.as_str())
    }

    fn text_of(replacement: &TextReplacement) -> Option<&TextReplacement> {
        Some(replacement)
    }
}

#[cfg(feature = "tabular")]
impl TextValued for Tabular {
    fn value(data: &TextData) -> &str {
        data.as_str()
    }

    fn replacement(text: TextReplacement) -> TabularReplacement {
        TabularReplacement::Cell(text)
    }

    fn unchanged(data: &TextData) -> TabularReplacement {
        TabularReplacement::Cell(TextReplacement::substituted(data.as_str()))
    }

    fn text_of(replacement: &TabularReplacement) -> Option<&TextReplacement> {
        match replacement {
            TabularReplacement::Cell(text) => Some(text),
            TabularReplacement::DropRow | TabularReplacement::DropColumn => None,
        }
    }
}

#[cfg(feature = "metadata")]
impl TextValued for Metadata {
    fn value(data: &MetadataData) -> &str {
        data.as_str()
    }

    fn replacement(text: TextReplacement) -> MetadataReplacement {
        MetadataReplacement::Value(text)
    }

    fn unchanged(_data: &MetadataData) -> MetadataReplacement {
        MetadataReplacement::Unchanged
    }

    fn text_of(replacement: &MetadataReplacement) -> Option<&TextReplacement> {
        match replacement {
            MetadataReplacement::Value(text) => Some(text),
            MetadataReplacement::Unchanged => None,
        }
    }
}

mod sealed {
    pub trait Sealed {}

    impl Sealed for elide_core::modality::text::Text {}
    #[cfg(feature = "tabular")]
    impl Sealed for elide_core::modality::tabular::Tabular {}
    #[cfg(feature = "metadata")]
    impl Sealed for elide_core::modality::metadata::Metadata {}
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "metadata")]
    use elide_core::entity::LabelRef;

    use super::*;

    #[test]
    fn text_is_its_own_replacement() {
        let removed = Text::replacement(TextReplacement::Removed);
        assert_eq!(removed, TextReplacement::Removed);
        assert_eq!(Text::text_of(&removed), Some(&TextReplacement::Removed));
        assert_eq!(
            Text::unchanged(&TextData::new("x")),
            TextReplacement::substituted("x")
        );
    }

    #[cfg(feature = "tabular")]
    #[test]
    fn a_table_cell_wraps_its_text() {
        let cell = Tabular::replacement(TextReplacement::Removed);
        assert_eq!(cell, TabularReplacement::Cell(TextReplacement::Removed));
        assert_eq!(Tabular::text_of(&cell), Some(&TextReplacement::Removed));
        assert_eq!(Tabular::text_of(&TabularReplacement::DropRow), None);
    }

    #[cfg(feature = "metadata")]
    #[test]
    fn a_metadata_field_keeps_its_value_unchanged() {
        let data = MetadataData::new("Artist", "Jane", LabelRef::new("person"));
        assert_eq!(Metadata::unchanged(&data), MetadataReplacement::Unchanged);
        assert_eq!(Metadata::text_of(&MetadataReplacement::Unchanged), None);
        assert_eq!(Metadata::value(&data), "Jane");
    }
}
