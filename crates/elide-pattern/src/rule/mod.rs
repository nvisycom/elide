//! The rules a [`PatternRecognizer`] runs, one module per rule type.
//!
//! Each holds the rule as authored (TOML or a builder) and its compiled form:
//! a [`Pattern`] with its [`Variant`]s, and a [`Dictionary`] of [`Term`]s.
//!
//! [`PatternRecognizer`]: crate::PatternRecognizer

mod dictionary;
mod pattern;

pub(crate) use self::dictionary::CompiledDictionary;
pub use self::dictionary::{Dictionary, DictionaryBuilder, Scoring, Term};
pub(crate) use self::pattern::CompiledPattern;
pub use self::pattern::{Pattern, PatternBuilder, Variant};
