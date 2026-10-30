//! Keyword-based confidence boosting for recognized entities.
//!
//! Some matches are ambiguous on their own but become far more likely when the
//! surrounding text gives them away. A nine-digit number is just a number until
//! the word "SSN" sits beside it. The [`Enhancer`] raises the confidence of an
//! entity when configured keywords appear nearby, so weak-but-plausible findings
//! clear the threshold only when their context supports them.
//!
//! Each [`BoostRule`] declares the keywords a label cares about and how close
//! they need to be. After recognition runs, the [`Enhancer`] scans the
//! neighbourhood of each entity for those keywords and lifts its confidence on a
//! hit. It matches keywords as plain substrings ([`SubstringMatcher`]), or, when
//! token information is available, across word variants such as "running" and
//! "run" ([`LemmaMatcher`]). Each boost is recorded on the entity, so its effect
//! on the final confidence is always traceable.
//!
//! [`PatternRecognizer`] builds its rules from every pattern's and dictionary's
//! `[context]` table and applies them to its own matches. A custom recognizer
//! can hold an [`Enhancer`] and call [`Enhancer::apply`] on its entities.
//!
//! [`SubstringMatcher`]: matching::SubstringMatcher
//! [`LemmaMatcher`]: matching::LemmaMatcher
//! [`PatternRecognizer`]: crate::PatternRecognizer

mod enhancer;
pub mod matching;
mod rule;

pub use self::enhancer::{Boost, EnhanceInput, Enhancer};
pub use self::rule::{
    Anchor, BoostRule, DEFAULT_BOOST, DEFAULT_PREFIX_WORDS, DEFAULT_SUFFIX_WORDS,
};
