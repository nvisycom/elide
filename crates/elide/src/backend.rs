//! Backends: the model-call contract and its decorators.
//!
//! Every hosted model in the toolkit — LLM/VLM, NER, speech-to-text, OCR — is a
//! [`Backend`]: request in, model call, response out. A recognizer or enricher
//! holds one and drives it. The contract ([`Backend`], [`BackendRequest`],
//! [`BackendResponse`], and the [`Units`] / [`Meter`] / [`TokenCounts`] a response
//! reports) is always available.
//!
//! The decorators wrap a `Backend`, leaving its request and response untouched.
//! The [`metrics`] module (feature `metrics`) reports each call's usage — a
//! `Metered` wrapper routes it to a per-run collector, or onward to a
//! process-wide system (features `metrics-recorder`, `metrics-tracing`). The
//! [`limit`] module bounds how a provider is called — `BatchChunked` caps the
//! batch size, and with feature `ratelimit`, `ConcurrencyLimited` caps
//! simultaneous calls and `RateLimited` caps calls per interval. The [`routing`]
//! module (feature `routing`) picks which backend serves a call —
//! `LanguageRouted` by the request's language, `FallbackRouted` by whether the
//! primary failed. [`BackendExt`] applies any of them by method. All re-exported
//! from [`elide_backend`]; `limit` and `BackendExt` come with any of the three
//! features.

#[cfg(feature = "metrics")]
#[cfg_attr(docsrs, doc(cfg(feature = "metrics")))]
#[doc(inline)]
pub use elide_backend::metrics;
#[cfg(feature = "routing")]
#[cfg_attr(docsrs, doc(cfg(feature = "routing")))]
#[doc(inline)]
pub use elide_backend::routing;
#[cfg(any(feature = "metrics", feature = "ratelimit", feature = "routing"))]
#[cfg_attr(
    docsrs,
    doc(cfg(any(feature = "metrics", feature = "ratelimit", feature = "routing")))
)]
#[doc(inline)]
pub use elide_backend::{BackendExt, limit};
#[doc(inline)]
pub use elide_core::backend::{
    Backend, BackendRequest, BackendResponse, Meter, TokenCounts, Units,
};
