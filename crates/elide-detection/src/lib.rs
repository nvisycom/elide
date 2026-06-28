#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod analyzer;
mod layer;

pub use self::analyzer::Analyzer;
pub use self::layer::{Layer, LayerOutput, calibrate, filter, reconcile};
