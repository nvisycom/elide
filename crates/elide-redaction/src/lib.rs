#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod anonymizer;
mod deanonymizer;
pub mod redaction;

pub use self::anonymizer::Anonymizer;
pub use self::deanonymizer::Deanonymizer;
