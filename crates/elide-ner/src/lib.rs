#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod backend;
#[cfg(feature = "lingua")]
pub mod nlp;
mod recognition;

pub use self::recognition::{
    LabelMap, NerModel, NerModelBuilder, NerRecognizer, NerRecognizerBuilder,
};
