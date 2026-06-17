#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod recognition;
mod shipped;
pub mod validators;

pub use self::recognition::{
    Context, Dictionary, DictionaryBuilder, PatternRecognizer, PatternRecognizerBuilder, Regex,
    RegexBuilder, Scoring, Term, Variant,
};
pub use self::shipped::{dictionaries, patterns};
