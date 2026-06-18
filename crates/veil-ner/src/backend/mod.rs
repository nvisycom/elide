//! Backend layer: the [`NerBackend`] trait and its shipped impls.
//!
//! One trait covers zero-shot backends (per-call labels via
//! [`NerRequest::labels`] = `Some(...)`) and fixed-label backends
//! (labels baked into the model, `labels = None`). The `mock`-gated
//! [`MockBackend`] (returns no spans; test/example stub) ships here;
//! concrete inference backends live downstream.

#[cfg(any(test, feature = "mock"))]
mod mock_backend;
mod ner_backend;
mod ner_span;

#[cfg(any(test, feature = "mock"))]
#[cfg_attr(docsrs, doc(cfg(feature = "mock")))]
pub use self::mock_backend::MockBackend;
pub use self::ner_backend::{NerBackend, NerRequest, NerResponse};
pub use self::ner_span::RawNerSpan;
