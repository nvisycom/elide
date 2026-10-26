//! [`Text`] modality: plain or structured text addressed by byte ranges.

mod data;
mod location;
mod replacement;
mod source_ref;
#[cfg(feature = "fixtures")]
mod test_doc;
mod tokens;

use std::ops::Range;

pub use self::data::TextData;
pub use self::location::{Detected, Marked, TextLocation};
pub use self::replacement::TextReplacement;
pub use self::source_ref::SourceRef;
#[cfg(feature = "fixtures")]
#[cfg_attr(docsrs, doc(cfg(feature = "fixtures")))]
pub use self::test_doc::TextDoc;
pub use self::tokens::{Token, Tokens};
use super::Modality;
use super::text_recognizable::TextRecognizable;
use crate::entity::Entity;

/// Text modality: data is [`TextData`], locations are
/// [`TextLocation`] byte ranges, replacements are [`TextReplacement`].
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Text;

impl Entity<Text> {
    /// Translate this entity's text coordinates forward by `delta` bytes: both
    /// its [`location`](Entity::location) span and its
    /// [`recognized_range`](Entity::recognized_range). Returns `None` when the
    /// location has no span to move (a reviewer mark).
    ///
    /// This rebases a finding made over a *sub-window* of a larger text onto the
    /// window's start in the full text. The window is a byte-slice of that text,
    /// so the window-local and full-text coordinate spaces differ only by this
    /// offset, and **both** fields move: `location` is the geometric coordinate,
    /// and `recognized_range` keys back into the recognized text — which here is
    /// that same full text — so it must name the same space the rest of the
    /// subject's spans and language claims are in.
    ///
    /// This is unlike a chunk-to-source lift, which moves only `location`: there
    /// the recognized text (an OCR layout, a transcript) is a genuinely different
    /// coordinate space from the source document, so `recognized_range` stays put
    /// as the artifact key. The [`audit`](Entity::audit) trail is likewise left
    /// untouched — it records the recognizer's own window-local view, which is
    /// genuinely where the match was reported.
    #[must_use]
    pub fn offset(mut self, delta: usize) -> Option<Self> {
        self.location = self.location.offset(delta)?;
        if let Some(range) = &mut self.recognized_range {
            *range = range.start + delta..range.end + delta;
        }
        Some(self)
    }
}

impl Modality for Text {
    type Artifact = Tokens;
    type Data = TextData;
    type Location = TextLocation;
    type Replacement = TextReplacement;

    const NAME: &'static str = "text";
}

impl TextRecognizable for Text {
    fn as_text<'a>(data: &'a TextData, _artifact: Option<&'a Tokens>) -> Option<&'a str> {
        Some(data.text.as_str())
    }

    fn locate(
        range: Range<usize>,
        _data: &TextData,
        _artifact: Option<&Tokens>,
    ) -> Option<TextLocation> {
        Some(TextLocation::inline(range))
    }

    fn as_tokens(artifact: Option<&Tokens>) -> Option<&[Token]> {
        artifact.filter(|t| !t.is_empty()).map(Tokens::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{EntityBuilder, LabelRef};
    use crate::redaction::Redactions;

    fn text_entity(location: TextLocation) -> Entity<Text> {
        EntityBuilder::new()
            .with_label(LabelRef::new("US_SSN"))
            .with_location(location)
            .build()
            .expect("label and location provided")
    }

    #[test]
    fn entity_offset_shifts_location_and_recognized_range_together() {
        let entity = EntityBuilder::new()
            .with_label(LabelRef::new("US_SSN"))
            .with_location(TextLocation::inline(2..5))
            .with_recognized_range(2..5)
            .build()
            .expect("label and location provided");
        let moved = entity.offset(100).expect("inline location has a span");
        assert_eq!(moved.location.range(), Some(&(102..105)));
        assert_eq!(moved.recognized_range, Some(102..105));
    }

    #[test]
    fn entity_offset_of_a_marked_location_is_none() {
        // A reviewer mark has no span, so there is nothing to rebase.
        let entity = text_entity(TextLocation::marked_source(SourceRef::new(0..5)));
        assert!(entity.offset(10).is_none());
    }

    #[test]
    fn sort_by_position_orders_in_place() {
        let mut batch: Redactions<Text> = Redactions::new();
        // Pushed out of order.
        batch.push(TextLocation::inline(20..25), TextReplacement::Removed);
        batch.push(TextLocation::inline(0..5), TextReplacement::Removed);
        batch.push(TextLocation::inline(10..15), TextReplacement::Removed);

        batch.sort_by_position();

        let starts: Vec<usize> = batch
            .iter()
            .map(|(loc, _)| loc.range().unwrap().start)
            .collect();
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
