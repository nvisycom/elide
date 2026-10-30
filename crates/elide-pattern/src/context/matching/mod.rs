//! Keyword-matching strategies for a rule's context.
//!
//! - [`KeywordMatcher`] is the strategy trait, selected with
//!   [`PatternRecognizerBuilder::with_keyword_matcher`].
//! - [`SubstringMatcher`] is the default: ASCII case-insensitive
//!   substring search over the raw text window.
//! - [`LemmaMatcher`] reads the lemmas of the tokens a tokenizing
//!   enricher stored as the subject's artifact. It recognizes
//!   morphological variants substring matching misses.
//!
//! [`PatternRecognizerBuilder::with_keyword_matcher`]: crate::PatternRecognizerBuilder::with_keyword_matcher

mod lemma;
mod substring;

use std::fmt::Debug;
use std::ops::Range;

use elide_core::modality::text::Token;
use hipstr::HipStr;

pub use self::lemma::LemmaMatcher;
pub use self::substring::SubstringMatcher;

/// Whether a keyword at the byte range `m` of `text` is a whole word: the
/// characters immediately before and after are not word characters. A
/// word character is a Unicode alphanumeric; `_` and `-` are treated as
/// separators, so a `snake_case` / `kebab-case` identifier tokenizes into
/// its words: the keyword `"postal"` matches inside the column header
/// `"postal_code"`, and `"tax"` inside `"tax-id"`. Otherwise this mirrors
/// a regex `\b…\b`, so `"AUD"` matches the token `AUD` but not the `aud`
/// inside `audit`, and `"karte"` does not match inside `"Kreditkarte"`.
pub(crate) fn is_whole_word(text: &str, m: &Range<usize>) -> bool {
    let is_word = |c: char| c.is_alphanumeric();
    let before_ok = text[..m.start]
        .chars()
        .next_back()
        .is_none_or(|c| !is_word(c));
    let after_ok = text[m.end..].chars().next().is_none_or(|c| !is_word(c));
    before_ok && after_ok
}

/// Finds where keywords fire near an entity match.
///
/// The strategy slot that swaps raw substring
/// matching for lemma-aware matching (or a third-party
/// fuzzy/word-boundary implementation) without changing its core
/// pipeline.
///
/// Implementations receive both a raw `window` slice of the source
/// text (for substring strategies) and the `tokens` covering that
/// same range (for token/lemma strategies). Either or both may be
/// ignored; `tokens` is empty when no NLP engine produced a token
/// artifact.
///
/// A matcher reports *every* candidate hit rather than just the first:
/// the recognizer applies the rule's word-boundary policy over the
/// candidates, so a matcher that stopped at the
/// first raw hit could mask a later boundary-valid one, e.g. `"karte"`
/// inside `"Kreditkarte"` must not hide the whole-word `"kreditkarte"`.
/// Keeping the boundary decision in one place (the recognizer) also means
/// each matcher stays a pure "where does this keyword occur" strategy.
pub trait KeywordMatcher: Debug + Send + Sync {
    /// The window-relative byte ranges of every keyword occurrence, in
    /// scan order. Empty when none fire. The recognizer offsets a chosen
    /// range into stream coordinates to resolve a location.
    fn matches(
        &self,
        window: &str,
        tokens: &[Token],
        keywords: &[HipStr<'static>],
    ) -> Vec<Range<usize>>;
}
