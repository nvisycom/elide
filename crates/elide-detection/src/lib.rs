#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod analyzer;
mod deduplication;

pub use self::analyzer::Analyzer;
pub use self::deduplication::{Layer, LayerOutput, calibrate, filter, fuse, resolve};
