#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod backend;
pub mod decorator;
mod recognition;

pub use self::recognition::{
    AggregationStrategy, AlignmentMode, NerRecognizer, NerRecognizerBuilder,
};
