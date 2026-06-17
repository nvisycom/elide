//! Lemma-aware [`KeywordMatcher`] implementation.

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
    fn any_match(&self, window: &str, tokens: &[Token], keywords: &[HipStr<'static>]) -> bool {
        if tokens.is_empty() {
            return SubstringMatcher.any_match(window, tokens, keywords);
        }
        let lowered_keywords: Vec<String> = keywords
            .iter()
            .map(|k| k.as_str().to_ascii_lowercase())
            .collect();
        tokens.iter().any(|tok| {
            let lemma = tok.lemma.as_str().to_ascii_lowercase();
            lowered_keywords.contains(&lemma)
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
        assert!(m.any_match("", &tokens, &kws(&["run"])));
        assert!(m.any_match("", &tokens, &kws(&["dog"])));
        assert!(!m.any_match("", &tokens, &kws(&["cat"])));
    }

    #[test]
    fn falls_back_to_substring_without_tokens() {
        let m = LemmaMatcher;
        assert!(m.any_match("Your SSN: 123", &[], &kws(&["ssn"])));
    }
}
