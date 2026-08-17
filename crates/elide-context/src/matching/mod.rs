//! Keyword-matching strategies plugged into the [`Enhancer`].
//!
//! - [`KeywordMatcher`] is the trait the enhancer talks to.
//! - [`SubstringMatcher`] is the default: ASCII case-insensitive
//!   substring search over the raw text window. Runs whenever no
//!   token artifact is present on `RecognizerContext.artifacts`.
//! - [`LemmaMatcher`] reads lemmatized tokens an upstream NLP
//!   engine stamped on `RecognizerContext.artifacts`. It recognizes
//!   morphological variants substring matching misses.
//!
//! [`Enhancer`]: crate::Enhancer

mod lemma;
mod substring;

use std::ops::Range;

use hipstr::HipStr;

pub use self::lemma::LemmaMatcher;
pub use self::substring::SubstringMatcher;
use crate::io::Token;

/// Whether the byte range `m` of `text` sits on word boundaries — the
/// characters immediately before and after are not word characters
/// (Unicode alphanumerics or `_`). Mirrors a regex `\b…\b` around a
/// keyword, so `"AUD"` matches the token `AUD` but not the `aud` inside
/// `audit`, and `"karte"` does not match inside `"Kreditkarte"`.
pub(crate) fn on_word_boundaries(text: &str, m: &Range<usize>) -> bool {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let before_ok = text[..m.start]
        .chars()
        .next_back()
        .is_none_or(|c| !is_word(c));
    let after_ok = text[m.end..].chars().next().is_none_or(|c| !is_word(c));
    before_ok && after_ok
}

/// Finds where keywords fire near an entity match.
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
///
/// A matcher reports *every* candidate hit rather than just the first:
/// the enhancer applies its own word-boundary policy over the
/// candidates (see [`Enhancer`]), so a matcher that stopped at the
/// first raw hit could mask a later boundary-valid one — e.g. `"karte"`
/// inside `"Kreditkarte"` must not hide the whole-word `"kreditkarte"`.
/// Keeping the boundary decision in one place (the enhancer) also means
/// each matcher stays a pure "where does this keyword occur" strategy.
///
/// [`Enhancer`]: crate::Enhancer
pub trait KeywordMatcher: Send + Sync {
    /// The window-relative byte ranges of every keyword occurrence, in
    /// scan order. Empty when none fire. The enhancer offsets a chosen
    /// range into stream coordinates to resolve a location.
    fn matches(
        &self,
        window: &str,
        tokens: &[Token],
        keywords: &[HipStr<'static>],
    ) -> Vec<Range<usize>>;
}
