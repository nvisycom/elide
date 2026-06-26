#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod backend;
mod enricher;

#[cfg(any(test, feature = "mock"))]
#[cfg_attr(docsrs, doc(cfg(feature = "mock")))]
pub use self::backend::MockBackend;
pub use self::backend::{OcrBackend, OcrRequest, OcrResponse};
pub use self::enricher::OcrEnricher;
