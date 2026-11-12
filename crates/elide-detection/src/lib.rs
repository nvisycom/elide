#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod analyzer;
mod batch;
mod layer;

pub mod decorator;

pub use self::analyzer::{Analysis, Analyzer};
pub use self::layer::{Layer, LayerOutput, calibrate, filter, overlap};
