//! [`KeywordMatcher`] trait + the default [`SubstringMatcher`].

use std::ops::Range;

use hipstr::HipStr;

use crate::io::Token;

/// Decides whether any keyword fires near an entity match, and where.
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
    /// The byte range, **within `window`**, of the first keyword that
    /// fires, or `None` when none do. The range is window-relative; the
    /// caller offsets it into stream coordinates to resolve a location.
    fn any_match(
        &self,
        window: &str,
        tokens: &[Token],
        keywords: &[HipStr<'static>],
    ) -> Option<Range<usize>>;
}

/// ASCII case-insensitive substring matcher.
///
/// The default matcher. It runs whenever no token artifact was
/// stamped on `RecognizerContext.artifacts`, or whenever the caller
/// explicitly picks raw matching.
///
/// Fast, allocation-light, permissive: the keyword `"email"` fires
/// inside `"MyEmailAddress"`. Ignores the `tokens` argument.
#[derive(Debug, Clone, Copy, Default)]
pub struct SubstringMatcher;

impl KeywordMatcher for SubstringMatcher {
    fn any_match(
        &self,
        window: &str,
        _tokens: &[Token],
        keywords: &[HipStr<'static>],
    ) -> Option<Range<usize>> {
        // `to_ascii_lowercase` rewrites bytes in place without changing
        // length, so an offset into `lowered` is the same offset into
        // `window` — the match position is reusable as-is.
        let lowered = window.to_ascii_lowercase();
        keywords.iter().find_map(|kw| {
            let needle = kw.as_str().to_ascii_lowercase();
            lowered
                .find(&needle)
                .map(|start| start..start + needle.len())
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
    fn substring_matches_case_insensitively() {
        let m = SubstringMatcher;
        // "SSN" sits at bytes 5..8 of the window.
        assert_eq!(
            m.any_match("Your SSN: 123", &[], &kws(&["ssn"])),
            Some(5..8)
        );
        assert_eq!(
            m.any_match(
                "the SOCIAL SECURITY number",
                &[],
                &kws(&["social security"])
            ),
            Some(4..19)
        );
        assert_eq!(m.any_match("nothing here", &[], &kws(&["ssn"])), None);
    }

    #[test]
    fn substring_is_permissive() {
        let m = SubstringMatcher;
        // "Email" inside "MyEmailAddress" is bytes 2..7.
        assert_eq!(
            m.any_match("MyEmailAddress", &[], &kws(&["email"])),
            Some(2..7)
        );
    }
}
