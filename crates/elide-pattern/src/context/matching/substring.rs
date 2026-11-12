//! The default [`SubstringMatcher`].
//!
//! [`SubstringMatcher`]: SubstringMatcher

use std::ops::Range;

use elide_core::modality::text::Token;

use super::{KeywordMatcher, KeywordSet};

/// A [`KeywordMatcher`] that finds keywords as case-insensitive substrings.
///
/// The default matcher. Fast and permissive: the keyword `"email"` fires
/// inside `"MyEmailAddress"`, and the rule's word-boundary policy decides
/// whether that counts. Ignores the `tokens` argument.
#[derive(Debug, Clone, Copy, Default)]
pub struct SubstringMatcher;

impl KeywordMatcher for SubstringMatcher {
    fn matches(&self, window: &str, _tokens: &[Token], keywords: &KeywordSet) -> Vec<Range<usize>> {
        keywords.find_all(window)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kws(items: &[&'static str]) -> KeywordSet {
        KeywordSet::new(items.iter().copied()).unwrap()
    }

    #[test]
    fn matches_case_insensitively() {
        let m = SubstringMatcher;
        // "SSN" sits at bytes 5..8 of the window.
        assert_eq!(m.matches("Your SSN: 123", &[], &kws(&["ssn"])), vec![5..8]);
        assert_eq!(
            m.matches(
                "the SOCIAL SECURITY number",
                &[],
                &kws(&["social security"])
            ),
            vec![4..19]
        );
        assert!(m.matches("nothing here", &[], &kws(&["ssn"])).is_empty());
    }

    #[test]
    fn is_permissive() {
        let m = SubstringMatcher;
        // "Email" inside "MyEmailAddress" is bytes 2..7, the raw matcher
        // reports it; the boundary policy decides whether it counts.
        assert_eq!(
            m.matches("MyEmailAddress", &[], &kws(&["email"])),
            vec![2..7]
        );
    }

    #[test]
    fn reports_every_occurrence_of_a_keyword() {
        let m = SubstringMatcher;
        // "card" inside "cardholder" (0..4) and standalone (11..15): both are
        // reported so the boundary filter can skip the first and keep the second.
        assert_eq!(
            m.matches("cardholder card", &[], &kws(&["card"])),
            vec![0..4, 11..15]
        );
    }

    #[test]
    fn reports_hits_across_all_keywords() {
        let m = SubstringMatcher;
        // "karte" (inside "Kreditkarte") and the whole "kreditkarte" both
        // surface; the boundary filter later keeps only the latter.
        assert_eq!(
            m.matches("die Kreditkarte hier", &[], &kws(&["karte", "kreditkarte"])),
            // Sorted by position: "kreditkarte" (4..15) precedes "karte" (10..15).
            vec![4..15, 10..15]
        );
    }
}
