//! [`Text`] modality: plain or structured text addressed by byte ranges.

mod data;
mod location;
mod replacement;
mod source_ref;
#[cfg(feature = "test-util")]
mod test_doc;
mod tokens;

use std::ops::Range;

pub use self::data::TextData;
pub use self::location::TextLocation;
pub use self::replacement::TextReplacement;
pub use self::source_ref::SourceRef;
#[cfg(feature = "test-util")]
#[cfg_attr(docsrs, doc(cfg(feature = "test-util")))]
pub use self::test_doc::TextDoc;
pub use self::tokens::{Token, Tokens};
use super::Modality;
use super::text_recognizable::TextRecognizable;

/// Text modality: data is [`TextData`], locations are
/// [`TextLocation`] byte ranges, replacements are [`TextReplacement`].
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Text;

impl Modality for Text {
    type Artifact = Tokens;
    type Data = TextData;
    type Location = TextLocation;
    type Replacement = TextReplacement;

    const NAME: &'static str = "text";
}

impl TextRecognizable for Text {
    fn as_text<'a>(data: &'a TextData, _artifact: Option<&'a Tokens>) -> &'a str {
        data.text.as_str()
    }

    fn locate(
        range: Range<usize>,
        _data: &TextData,
        _artifact: Option<&Tokens>,
    ) -> Option<TextLocation> {
        Some(TextLocation::new(range.start, range.end))
    }

    fn as_tokens(artifact: Option<&Tokens>) -> Option<&[Token]> {
        artifact.filter(|t| !t.is_empty()).map(Tokens::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redaction::Redactions;

    #[test]
    fn sort_by_position_orders_in_place() {
        let mut batch: Redactions<Text> = Redactions::new();
        // Pushed out of order.
        batch.push(TextLocation::new(20, 25), TextReplacement::Removed);
        batch.push(TextLocation::new(0, 5), TextReplacement::Removed);
        batch.push(TextLocation::new(10, 15), TextReplacement::Removed);

        batch.sort_by_position();

        let starts: Vec<usize> = batch.iter().map(|(loc, _)| loc.range.start).collect();
        assert_eq!(starts, [0, 10, 20]);
    }

    #[test]
    fn as_tokens_exposes_producer_lemmas() {
        // A producer whose lemma differs from the surface form: the context
        // enhancer reads `as_tokens` to boost on the lemma, so the differing
        // lemma must survive the artifact -> tokens hop.
        let tokens = Tokens::new(vec![
            Token::from_text("running", 0..7).with_lemma("run"),
            Token::from_text("dogs", 8..12).with_lemma("dog"),
        ]);
        let seen = Text::as_tokens(Some(&tokens)).expect("text carries its tokens");
        let lemmas: Vec<&str> = seen.iter().map(|t| t.lemma.as_str()).collect();
        assert_eq!(lemmas, ["run", "dog"]);
    }

    #[test]
    fn as_tokens_is_none_without_an_artifact() {
        // No tokenizing enricher ran (None), or one ran and produced nothing
        // (an empty artifact): either way, no tokens, context matching falls
        // back to the surface text.
        assert!(Text::as_tokens(None).is_none());
        assert!(Text::as_tokens(Some(&Tokens::default())).is_none());
    }
}
