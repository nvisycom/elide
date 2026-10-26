#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod analyzer;
mod chunked;
mod layer;
mod resilient;

pub mod escalation;

pub use self::analyzer::{Analysis, Analyzer};
pub use self::chunked::{CharWindows, ChunkedRecognizer, Chunking, Window, WordWindows};
pub use self::layer::{Layer, LayerOutput, calibrate, filter, reconcile};
pub use self::resilient::ResilientRecognizer;
