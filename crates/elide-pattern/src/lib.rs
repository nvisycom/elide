#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod context;
mod recognizer;
mod rule;
mod shipped;
pub mod validators;

pub use self::recognizer::{PatternRecognizer, PatternRecognizerBuilder, RegexBudget};
pub use self::rule::{
    Dictionary, DictionaryBuilder, Direction, Lowercase, Pattern, PatternBuilder, Scoring, Term,
    Variant, Walk, WalkBuilder, Words,
};
pub use self::shipped::{dictionaries, patterns, walks};
