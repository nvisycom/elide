#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod backend;
mod enricher;

#[cfg(any(test, feature = "test-utils"))]
#[cfg_attr(docsrs, doc(cfg(feature = "test-utils")))]
pub use self::backend::MockBackend;
#[cfg(feature = "ocrs")]
#[cfg_attr(docsrs, doc(cfg(feature = "ocrs")))]
pub use self::backend::{OCRS_MODELS_DIR_ENV, OcrsBackend};
pub use self::backend::{OcrBackend, OcrRequest, OcrResponse};
pub use self::enricher::{OcrEnricher, OcrEnricherBuilder};
