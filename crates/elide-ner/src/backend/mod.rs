//! NER backend types: the [`NerRequest`]/[`NerResponse`] a NER backend speaks,
//! the decorators that shape a model's labels and scores, and the shipped
//! impls.
//!
//! A NER backend is a [`Backend`](elide_core::backend::Backend) whose request
//! carries `(text, labels)` and whose response carries canonical [`NerSpan`]s. It
//! covers zero-shot backends (per-call labels via [`NerRequest::labels`] =
//! `Some(...)`) and fixed-label backends (labels baked into the model, `labels =
//! None`).
//!
//! Which labels a model detects, which it emits but should be dropped, and how
//! its scores compare are properties of the model, so they are decorators over
//! its backend rather than recognizer settings: [`LabelScoped`] scopes it to the
//! labels it supports, [`LabelExcluded`] drops labels it emits, and
//! [`ScoreScaled`] rescales chosen labels' scores. [`NerBackendExt`] applies
//! them by method. The `mocks`-gated [`MockBackend`] (returns no spans;
//! test/example stub) ships here; concrete inference backends live downstream.

mod ext;
mod label_excluded;
mod label_scoped;
#[cfg(any(test, feature = "mocks"))]
mod mock_backend;
mod ner_request;
mod ner_response;
mod score_scaled;

pub use self::ext::NerBackendExt;
pub use self::label_excluded::LabelExcluded;
pub use self::label_scoped::LabelScoped;
#[cfg(any(test, feature = "mocks"))]
#[cfg_attr(docsrs, doc(cfg(feature = "mocks")))]
pub use self::mock_backend::MockBackend;
pub use self::ner_request::NerRequest;
pub use self::ner_response::{NerResponse, NerSpan};
pub use self::score_scaled::ScoreScaled;
