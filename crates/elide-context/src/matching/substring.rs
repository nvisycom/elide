//! The default [`SubstringMatcher`].
//!
//! [`SubstringMatcher`]: SubstringMatcher

use std::ops::Range;

use elide_core::modality::text::Token;
use hipstr::HipStr;

use super::KeywordMatcher;

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
    fn matches(
        &self,
        window: &str,
        _tokens: &[Token],
        keywords: &[HipStr<'static>],
    ) -> Vec<Range<usize>> {
        // `to_ascii_lowercase` rewrites bytes in place without changing
        // length, so an offset into `lowered` is the same offset into
        // `window`, each match position is reusable as-is.
        let lowered = window.to_ascii_lowercase();
        let mut matches: Vec<Range<usize>> = keywords
            .iter()
            .flat_map(|kw| {
                let needle = kw.as_str().to_ascii_lowercase();
                // Every occurrence, so the enhancer's boundary filter can skip
                // a substring hit and still reach a later boundary-valid one.
                lowered
                    .match_indices(&needle)
                    .map(|(start, _)| start..start + needle.len())
                    .collect::<Vec<_>>()
            })
            .collect();
        // Report in text-scan order (by position), not grouped by keyword, so
        // the enhancer resolves the earliest matching keyword's location.
        matches.sort_by_key(|range| range.start);
        matches
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kws(items: &[&'static str]) -> Vec<HipStr<'static>> {
        items.iter().copied().map(HipStr::from).collect()
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
        // reports it; the enhancer's boundary policy decides whether it counts.
        assert_eq!(
            m.matches("MyEmailAddress", &[], &kws(&["email"])),
            vec![2..7]
        );
    }

    #[test]
    fn reports_every_occurrence_of_a_keyword() {
        let m = SubstringMatcher;
        // "card" inside "cardholder" (0..4) and standalone (11..15): both are
        // reported so the enhancer can skip the first and keep the second.
        assert_eq!(
            m.matches("cardholder card", &[], &kws(&["card"])),
            vec![0..4, 11..15]
        );
    }

    #[test]
    fn reports_hits_across_all_keywords() {
        let m = SubstringMatcher;
        // "karte" (inside "Kreditkarte") and the whole "kreditkarte" both
        // surface; the enhancer's boundary filter later keeps only the latter.
        assert_eq!(
            m.matches("die Kreditkarte hier", &[], &kws(&["karte", "kreditkarte"])),
            // Sorted by position: "kreditkarte" (4..15) precedes "karte" (10..15).
            vec![4..15, 10..15]
        );
    }
}
