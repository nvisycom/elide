//! The rules a [`PatternRecognizer`] runs, one module per rule type.
//!
//! Each holds the rule as authored (TOML or a builder) and its compiled form:
//! a [`Pattern`] with its [`Variant`]s, a [`Dictionary`] of [`Term`]s, and a
//! [`Walk`] from an anchor back to the start of a name.
//!
//! [`PatternRecognizer`]: crate::PatternRecognizer

mod dictionary;
mod pattern;
mod walk;

pub(crate) use self::dictionary::CompiledDictionary;
pub use self::dictionary::{Dictionary, DictionaryBuilder, Scoring, Term};
pub(crate) use self::pattern::CompiledPattern;
pub use self::pattern::{Pattern, PatternBuilder, Variant};
pub(crate) use self::walk::CompiledWalk;
pub use self::walk::{Direction, Lowercase, Walk, WalkBuilder, Words};
