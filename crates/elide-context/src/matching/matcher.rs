//! [`KeywordMatcher`] trait + the default [`SubstringMatcher`].

use hipstr::HipStr;

use crate::io::Token;

/// Decide whether any keyword from `keywords` fires within the
/// candidate region around an entity match.
///
/// The strategy slot that lets the enhancer swap raw substring
/// matching for lemma-aware matching (or a third-party
/// fuzzy/word-boundary implementation) without changing its core
/// pipeline.
///
/// Implementations receive both a raw `window` slice of the source
/// text (for substring strategies) and the `tokens` covering that
/// same range (for token/lemma strategies). Either or both may be
/// ignored; `tokens` is empty when no NLP engine produced a token
/// artifact.
pub trait KeywordMatcher: Send + Sync {
    /// `true` if at least one keyword from `keywords` appears in
    /// the input.
    fn any_match(&self, window: &str, tokens: &[Token], keywords: &[HipStr<'static>]) -> bool;
}

/// ASCII case-insensitive substring matcher. The default —
/// runs whenever no token artifact was stamped on
/// `RecognizerInput.artifacts`, or whenever the caller explicitly
/// picks raw matching.
///
/// Fast, allocation-light, permissive: the keyword `"email"` fires
/// inside `"MyEmailAddress"`. Ignores the `tokens` argument.
#[derive(Debug, Clone, Copy, Default)]
pub struct SubstringMatcher;

impl KeywordMatcher for SubstringMatcher {
    fn any_match(&self, window: &str, _tokens: &[Token], keywords: &[HipStr<'static>]) -> bool {
        let lowered = window.to_ascii_lowercase();
        keywords
            .iter()
            .any(|kw| lowered.contains(kw.as_str().to_ascii_lowercase().as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kws(items: &[&'static str]) -> Vec<HipStr<'static>> {
        items.iter().copied().map(HipStr::from).collect()
    }

    #[test]
    fn substring_matches_case_insensitively() {
        let m = SubstringMatcher;
        assert!(m.any_match("Your SSN: 123", &[], &kws(&["ssn"])));
        assert!(m.any_match(
            "the SOCIAL SECURITY number",
            &[],
            &kws(&["social security"])
        ));
        assert!(!m.any_match("nothing here", &[], &kws(&["ssn"])));
    }

    #[test]
    fn substring_is_permissive() {
        let m = SubstringMatcher;
        assert!(m.any_match("MyEmailAddress", &[], &kws(&["email"])));
    }
}
