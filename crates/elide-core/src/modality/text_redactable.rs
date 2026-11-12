//! [`TextRedactable`]: the modalities redacted as text.

use crate::modality::Modality;
use crate::modality::text::TextReplacement;

/// A modality redacted as text: plain text, a table cell, a metadata field.
///
/// Its data reads as a string, and a [`TextReplacement`] converts into its
/// replacement and back out of it, so one text [`Operator`] or [`Generator`]
/// (a mask, a hash, a fake value) serves every such modality through a single
/// implementation. Any modality whose types meet the bounds is one; there is
/// nothing to implement.
///
/// The redaction-side counterpart of [`TextRecognizable`], with a real
/// difference: an image is recognized as text through OCR, but redacted by
/// its pixels, so it is [`TextRecognizable`] and not `TextRedactable`.
///
/// [`Operator`]: crate::redaction::Operator
/// [`Generator`]: crate::redaction::Generator
/// [`TextRecognizable`]: crate::modality::TextRecognizable
pub trait TextRedactable:
    Modality<Data: AsRef<str>, Replacement: From<TextReplacement> + TryInto<TextReplacement>>
{
}

impl<M> TextRedactable for M where
    M: Modality<Data: AsRef<str>, Replacement: From<TextReplacement> + TryInto<TextReplacement>>
{
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "metadata")]
    use crate::modality::metadata::Metadata;
    #[cfg(feature = "tabular")]
    use crate::modality::tabular::Tabular;
    use crate::modality::text::Text;

    fn redactable<M: TextRedactable>() {}

    /// Every text-valued core modality qualifies through its types alone.
    #[test]
    fn the_text_valued_modalities_are_redactable_as_text() {
        redactable::<Text>();
        #[cfg(feature = "tabular")]
        redactable::<Tabular>();
        #[cfg(feature = "metadata")]
        redactable::<Metadata>();
    }
}
