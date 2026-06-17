//! Backend layer: the [`NerBackend`] trait and its shipped impls.
//!
//! One trait covers zero-shot backends (per-call labels via
//! [`NerRequest::labels`] = `Some(...)`) and fixed-label backends
//! (labels baked into the model, `labels = None`). The built-in
//! [`NoopBackend`] (returns no spans; test stub) ships here; concrete
//! inference backends live downstream.

mod ner_backend;
mod ner_span;
mod noop_backend;

pub use self::ner_backend::{NerBackend, NerRequest, NerResponse};
pub use self::ner_span::RawNerSpan;
pub use self::noop_backend::NoopBackend;
