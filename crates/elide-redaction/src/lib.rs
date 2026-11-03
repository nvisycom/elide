#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod anonymizer;
mod deanonymizer;

pub mod generator;
pub mod operators;
pub mod vault;

pub use self::anonymizer::{Anonymizer, MatchContext, Rule};
pub use self::deanonymizer::Deanonymizer;
