//! Lemma-aware [`KeywordMatcher`] implementation.

use std::ops::Range;

use elide_core::modality::text::Token;

use super::{KeywordMatcher, KeywordSet, SubstringMatcher, by_position};

/// A [`KeywordMatcher`] that matches keywords against token lemmas.
///
/// Finds every [`SubstringMatcher`] hit, so an inflected keyword (`ruled`)
/// or a phrase (`filed with`) still matches the text as written, plus each
/// token in `tokens` whose lemma equals a keyword, ignoring case.
/// Without `tokens` (no shared NLP artifact was produced), only the
/// substring hits apply.
///
/// Recognizes morphological variants the substring matcher cannot:
/// `"running" → "run"`, `"dogs" → "dog"`, `"SSNs" → "ssn"`.
#[derive(Debug, Clone, Copy, Default)]
pub struct LemmaMatcher;

impl KeywordMatcher for LemmaMatcher {
    fn matches(&self, window: &str, tokens: &[Token], keywords: &KeywordSet) -> Vec<Range<usize>> {
        let mut found = SubstringMatcher.matches(window, tokens, keywords);
        if tokens.is_empty() {
            return found;
        }
        // Tokens carry stream-relative offsets; the contract is a
        // *window*-relative range. The window spans from the first token
        // (see `token_span`), so subtracting its start rebases each match.
        let base = tokens[0].offset.start;
        found.extend(
            tokens
                .iter()
                .filter(|tok| keywords.contains(tok.lemma.as_str()))
                .map(|tok| tok.offset.start - base..tok.offset.end - base),
        );
        found.sort_by(by_position);
        found.dedup();
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kws(items: &[&'static str]) -> KeywordSet {
        KeywordSet::new(items.iter().copied()).unwrap()
    }

    #[test]
    fn matches_morph_variants() {
        let tokens = vec![
            Token::from_text("the", 0..3),
            Token::from_text("running", 4..11).with_lemma("run"),
            Token::from_text("dogs", 12..16).with_lemma("dog"),
        ];
        let m = LemmaMatcher;
        // Ranges are window-relative; here the first token starts at 0, so
        // they coincide with the stream offsets.
        assert_eq!(m.matches("", &tokens, &kws(&["run"])), vec![4..11]);
        assert_eq!(m.matches("", &tokens, &kws(&["dog"])), vec![12..16]);
        assert!(m.matches("", &tokens, &kws(&["cat"])).is_empty());
    }

    #[test]
    fn matches_the_text_as_written() {
        let window = "Judge Alito ruled, filed with";
        let tokens = vec![
            Token::from_text("Judge", 0..5),
            Token::from_text("Alito", 6..11),
            Token::from_text("ruled", 12..17).with_lemma("rule"),
            Token::from_text("filed", 19..24).with_lemma("file"),
            Token::from_text("with", 25..29),
        ];
        let m = LemmaMatcher;
        assert_eq!(m.matches(window, &tokens, &kws(&["ruled"])), vec![12..17]);
        // The substring inside `ruled`, which the recognizer's word-boundary
        // check drops, and the token whose lemma is `rule`.
        assert_eq!(
            m.matches(window, &tokens, &kws(&["rule"])),
            vec![12..17, 12..16]
        );
        assert_eq!(
            m.matches(window, &tokens, &kws(&["filed with"])),
            vec![19..29]
        );
    }

    #[test]
    fn match_range_is_rebased_to_the_window() {
        // First token starts at byte 100 in the stream; the window begins
        // there, so the match's window-relative range is offset by 100.
        let tokens = vec![
            Token::from_text("running", 100..107).with_lemma("run"),
            Token::from_text("dogs", 108..112).with_lemma("dog"),
        ];
        let m = LemmaMatcher;
        assert_eq!(m.matches("", &tokens, &kws(&["dog"])), vec![8..12]);
    }

    #[test]
    fn falls_back_to_substring_without_tokens() {
        let m = LemmaMatcher;
        assert_eq!(m.matches("Your SSN: 123", &[], &kws(&["ssn"])), vec![5..8]);
    }
}
