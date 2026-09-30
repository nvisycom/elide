//! NER backend types: the [`NerRequest`]/[`NerResponse`] a NER
//! [`Backend`](elide_core::backend::Backend) speaks, and its shipped impls.
//!
//! A NER backend is a `Backend` whose request carries `(text, labels)` and whose
//! response carries canonical [`NerSpan`]s. It covers zero-shot backends (per-call
//! labels via [`NerRequest::labels`] = `Some(...)`) and fixed-label backends
//! (labels baked into the model, `labels = None`). Wrap a backend with a
//! [`decorator`] to scale or drop selected labels. The `mocks`-gated
//! [`MockBackend`] (returns no spans; test/example stub) ships here; concrete
//! inference backends live downstream.
//!
//! [`decorator`]: crate::decorator
//! [`LabelMap`]: elide_core::recognition::LabelMap

#[cfg(any(test, feature = "mocks"))]
mod mock_backend;
mod ner_request;
mod ner_response;

use elide_core::backend::Backend;

#[cfg(any(test, feature = "mocks"))]
#[cfg_attr(docsrs, doc(cfg(feature = "mocks")))]
pub use self::mock_backend::MockBackend;
pub use self::ner_request::NerRequest;
pub use self::ner_response::{NerResponse, NerSpan};

/// A NER [`Backend`](elide_core::backend::Backend): `Backend<Request<'a> =
/// NerRequest<'a>, Response = NerResponse>`.
///
/// A shorthand bound for the recognizer and decorators to hold any NER backend
/// generically, and a place to document what a NER backend must be. Everything
/// that turns `(text, labels)` into canonical NER spans implements it by
/// implementing `Backend` with these associated types: externalised inference
/// services, local model wrappers, and the in-process no-op test stub.
pub trait NerBackend:
    for<'a> Backend<Request<'a> = NerRequest<'a>, Response = NerResponse>
{
}

impl<B> NerBackend for B where
    B: for<'a> Backend<Request<'a> = NerRequest<'a>, Response = NerResponse>
{
}
