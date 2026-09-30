//! NER backend types: the [`NerRequest`]/[`NerResponse`] a NER backend speaks,
//! and its shipped impls.
//!
//! A NER backend is a [`Backend`](elide_core::backend::Backend) whose request
//! carries `(text, labels)` and whose response carries canonical [`NerSpan`]s. It
//! covers zero-shot backends (per-call labels via [`NerRequest::labels`] =
//! `Some(...)`) and fixed-label backends (labels baked into the model, `labels =
//! None`). Wrap a backend with a [`decorator`] to scale or drop selected labels.
//! The `mocks`-gated [`MockBackend`] (returns no spans; test/example stub) ships
//! here; concrete inference backends live downstream.
//!
//! [`decorator`]: crate::decorator
//! [`LabelMap`]: elide_core::recognition::LabelMap

#[cfg(any(test, feature = "mocks"))]
mod mock_backend;
mod ner_request;
mod ner_response;

#[cfg(any(test, feature = "mocks"))]
#[cfg_attr(docsrs, doc(cfg(feature = "mocks")))]
pub use self::mock_backend::MockBackend;
pub use self::ner_request::NerRequest;
pub use self::ner_response::{NerResponse, NerSpan};
