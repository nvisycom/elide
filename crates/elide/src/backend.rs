//! Backends: the model-call contract and the decorators that wrap it.
//!
//! Every hosted model in the toolkit — LLM/VLM, NER, speech-to-text, OCR — is a
//! [`Backend`]: request in, model call, response out. A recognizer or enricher
//! holds one and drives it. The contract ([`Backend`], [`Request`],
//! [`Response`]) is always available, with [`BackendBatchExt::call_aligned`],
//! the batch call a caller makes, and [`BatchSplit`] / [`BatchMerge`] for a
//! batch served in parts.
//!
//! The decorators wrap a `Backend`, leaving its request and response untouched.
//! The [`limit`] module bounds how a provider is called — `BatchChunked` caps
//! the batch size, and with feature `ratelimit`, `ConcurrencyLimited` caps
//! simultaneous calls and `RateLimited` caps calls per interval. The
//! [`routing`] module (feature `routing`) picks which backend serves a call —
//! `LanguageRouted` by the request's language, `FallbackRouted` by whether the
//! primary failed. [`BackendExt`] applies any of them by method. All
//! re-exported from [`elide_backend`]; `limit` and `BackendExt` come with
//! either feature.

#[cfg(feature = "routing")]
#[cfg_attr(docsrs, doc(cfg(feature = "routing")))]
#[doc(inline)]
pub use elide_backend::routing;
#[cfg(any(feature = "ratelimit", feature = "routing"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "ratelimit", feature = "routing"))))]
#[doc(inline)]
pub use elide_backend::{BackendExt, limit};
#[doc(inline)]
pub use elide_core::backend::{
    Backend, BackendBatchExt, BatchMerge, BatchPart, BatchSplit, Request, Response,
};
