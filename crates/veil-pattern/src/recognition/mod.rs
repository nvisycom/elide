//! Recognition primitives.
//!
//! Holds the rule shapes ([`Regex`] + its [`Variant`]s,
//! [`Dictionary`]), their building blocks ([`Term`]), and the
//! runtime [`PatternRecognizer`] that compiles them into pooled
//! scanners. Per-rule and per-dictionary `context` keyword lists
//! are harvested by [`PatternRecognizerBuilder::build_context_enhanced`]
//! into a wrapping `ContextEnhanced` layer that lifts confidence
//! on matches near a declared keyword.

mod compiled;
mod context;
mod dictionary;
mod recognizer;
mod regex;
mod term;

pub use self::context::Context;
pub use self::dictionary::{Dictionary, DictionaryBuilder, Scoring};
pub use self::recognizer::{PatternRecognizer, PatternRecognizerBuilder};
pub use self::regex::{Regex, RegexBuilder, Variant};
pub use self::term::Term;
