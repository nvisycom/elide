//! OCR: recognize the text laid out in an image and enrich the call with a
//! [`Layout`](crate::modality::Layout).
//!
//! The [`OcrBackend`] trait covers every OCR engine (hosted document-AI APIs,
//! local engines, the in-process test stub); the [`OcrEnricher`] drives a
//! backend per call and stamps the recognized [`Layout`](crate::modality::Layout)
//! onto the image so a recognizer can read it. The pure-Rust `OcrsBackend` is
//! behind the `ocrs` feature; the no-op `MockBackend` behind `test-utils`.

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
