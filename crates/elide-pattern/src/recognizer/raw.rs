//! [`RawMatch`]: a rule's hit before it is placed as an entity.

use std::ops::Range;

use elide_core::entity::LabelRef;
use elide_core::entity::audit::PatternEvent;
use elide_core::primitive::Confidence;

/// One pattern/dictionary hit before placement: context lifts its
/// `confidence`, then the recognizer places it as a located [`Entity`] via
/// `M::locate(range)`, keeping `range` as the entity's `recognized_range`.
///
/// [`Entity`]: elide_core::entity::Entity
pub(crate) struct RawMatch {
    pub(crate) label: LabelRef,
    /// The rule's score for the match, as recorded in its pattern event.
    pub(crate) score: Confidence,
    /// The match's confidence: `score`, plus any context lift.
    pub(crate) confidence: Confidence,
    pub(crate) range: Range<usize>,
    pub(crate) pattern: PatternEvent,
    /// The rule's context, as an index into the recognizer's contexts.
    pub(crate) context: Option<usize>,
}

/// Whether a dictionary match at `text[start..end]` is a whole token, mirroring
/// regex `\b`: the immediate neighbour characters (or start/end of input) must
/// not be word characters. A word character here is Unicode alphanumeric or
/// `_`, matching the conventional regex definition, so a term never matches
/// inside an identifier like `my_term`. Context keywords use the looser
/// [`is_whole_word`](crate::context::matching), which splits on `_` and `-`.
///
/// Operates on `char` boundaries, not raw bytes, so multibyte
/// codepoints don't trigger false rejections (`é` is one char,
/// not two).
pub(crate) fn is_whole_token(text: &str, range: Range<usize>) -> bool {
    let left_is_word = text[..range.start]
        .chars()
        .next_back()
        .is_some_and(is_word_char);
    let right_is_word = text[range.end..].chars().next().is_some_and(is_word_char);
    !left_is_word && !right_is_word
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_whole_token_handles_edges_and_unicode() {
        // Match touches both edges of the input → boundaries OK.
        assert!(is_whole_token("hello", 0..5));
        // Match preceded by a word char → not a boundary.
        assert!(!is_whole_token("example", 5..7));
        // Match followed by a word char → not a boundary.
        assert!(!is_whole_token("amount", 0..2));
        // Space surround → boundaries OK.
        assert!(is_whole_token(" am ", 1..3));
        // Unicode word char on the left → not a boundary.
        assert!(!is_whole_token("café_am", 5..7));
        // Punctuation around → boundaries OK.
        assert!(is_whole_token("(am)", 1..3));
    }
}
