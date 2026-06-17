//! Backend layer: the [`NerBackend`] trait and its shipped impls.
//!
//! One trait covers zero-shot backends (per-call labels via
//! [`NerRequest::labels`] = `Some(...)`) and fixed-label backends
//! (labels baked into the model, `labels = None`). Built-in
//! [`NoopBackend`] (returns no spans; test stub) and feature-gated
//! [`BentoBackend`] (HTTP call into the externalised
//! `inference-gliner` service).

mod ner_backend;
mod ner_span;
mod noop_backend;

#[cfg(feature = "bento")]
mod bento_backend;
#[cfg(feature = "bento")]
mod bento_types;

#[cfg(feature = "bento")]
#[cfg_attr(docsrs, doc(cfg(feature = "bento")))]
pub use self::bento_backend::{BentoBackend, BentoParams};
pub use self::ner_backend::{NerBackend, NerRequest, NerResponse};
pub use self::ner_span::RawNerSpan;
pub use self::noop_backend::NoopBackend;
