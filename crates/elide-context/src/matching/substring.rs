//! The default [`SubstringMatcher`].
//!
//! [`SubstringMatcher`]: SubstringMatcher

use std::ops::Range;

use hipstr::HipStr;

use super::KeywordMatcher;
use crate::io::Token;

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
