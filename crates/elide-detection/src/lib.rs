#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod analyzer;
mod chunked;
#[cfg(feature = "metadata")]
mod field;
mod layer;
mod resilient;

pub mod escalation;

pub use self::analyzer::{Analysis, Analyzer};
pub use self::chunked::{CharWindows, ChunkedRecognizer, Chunking, Window, WordWindows};
#[cfg(feature = "metadata")]
pub use self::field::FieldRecognizer;
pub use self::layer::{Layer, LayerOutput, calibrate, filter, reconcile};
pub use self::resilient::ResilientRecognizer;
