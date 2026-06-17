#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod analyzer;
pub mod deduplication;

pub use self::analyzer::Analyzer;
