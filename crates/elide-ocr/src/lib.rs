#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod backend;
mod enricher;

pub use self::backend::{OcrBackend, OcrRequest, OcrResponse};
#[cfg(any(test, feature = "mock"))]
#[cfg_attr(docsrs, doc(cfg(feature = "mock")))]
pub use self::backend::MockBackend;
pub use self::enricher::OcrEnricher;
