//! Lemma-aware [`KeywordMatcher`] implementation.

use std::ops::Range;

use hipstr::HipStr;

use super::matcher::{KeywordMatcher, SubstringMatcher};
use crate::io::Token;

/// Lemma-aware matcher. Compares each lemma in `tokens` against
/// the keyword list with ASCII case-insensitive equality.
///
/// Falls back to [`SubstringMatcher`] semantics when `tokens` is
/// empty (no shared NLP artifact was produced) so the enhancer
/// runs uniformly regardless of whether the upstream pass emitted
/// tokens.
///
/// Recognizes morphological variants the substring matcher cannot:
/// `"running" → "run"`, `"dogs" → "dog"`, `"SSNs" → "ssn"`. Cost
/// is one lowercase per keyword + one lowercase per lemma per
/// match attempt.
#[derive(Debug, Clone, Copy, Default)]
pub struct LemmaMatcher;

impl KeywordMatcher for LemmaMatcher {
    fn any_match(
        &self,
        window: &str,
        tokens: &[Token],
        keywords: &[HipStr<'static>],
    ) -> Option<Range<usize>> {
        if tokens.is_empty() {
            return SubstringMatcher.any_match(window, tokens, keywords);
        }
        let lowered_keywords: Vec<String> = keywords
            .iter()
            .map(|k| k.as_str().to_ascii_lowercase())
            .collect();
        // Tokens carry stream-relative offsets; the contract is a
        // *window*-relative range. The window spans from the first token
        // (see `token_span`), so subtracting its start rebases each match.
        let base = tokens[0].offset.start;
        tokens.iter().find_map(|tok| {
            let lemma = tok.lemma.as_str().to_ascii_lowercase();
            lowered_keywords
                .contains(&lemma)
                .then(|| tok.offset.start - base..tok.offset.end - base)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kws(items: &[&'static str]) -> Vec<HipStr<'static>> {
        items.iter().copied().map(HipStr::from).collect()
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
        assert_eq!(m.any_match("", &tokens, &kws(&["run"])), Some(4..11));
        assert_eq!(m.any_match("", &tokens, &kws(&["dog"])), Some(12..16));
        assert_eq!(m.any_match("", &tokens, &kws(&["cat"])), None);
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
        assert_eq!(m.any_match("", &tokens, &kws(&["dog"])), Some(8..12));
    }

    #[test]
    fn falls_back_to_substring_without_tokens() {
        let m = LemmaMatcher;
        assert_eq!(
            m.any_match("Your SSN: 123", &[], &kws(&["ssn"])),
            Some(5..8)
        );
    }
}
