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
mod matcher;

pub use self::lemma::LemmaMatcher;
pub use self::matcher::{KeywordMatcher, SubstringMatcher};
