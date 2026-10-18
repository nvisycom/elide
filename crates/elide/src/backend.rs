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
//! [`ratelimit`] module (feature `ratelimit`) bounds how a provider is called —
//! `ConcurrencyLimited` caps simultaneous calls, `RateLimited` caps calls per
//! interval. Both re-exported from [`elide_backend`].

#[cfg(feature = "metrics")]
#[cfg_attr(docsrs, doc(cfg(feature = "metrics")))]
#[doc(inline)]
pub use elide_backend::metrics;
#[cfg(feature = "ratelimit")]
#[cfg_attr(docsrs, doc(cfg(feature = "ratelimit")))]
#[doc(inline)]
pub use elide_backend::ratelimit;
#[doc(inline)]
pub use elide_core::backend::{
    Backend, BackendRequest, BackendResponse, Meter, TokenCounts, Units,
};
