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
