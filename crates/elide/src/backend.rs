//! Backends: the model-call contract, and (under the `usage` feature) its usage
//! accounting.
//!
//! Every hosted model in the toolkit — LLM/VLM, NER, speech-to-text, OCR — is a
//! [`Backend`]: request in, model call, response out. A recognizer or enricher
//! holds one and drives it. The contract ([`Backend`], [`BackendRequest`],
//! [`BackendResponse`], and the [`Units`] / [`Meter`] / [`TokenCounts`] a response
//! reports) is always available.
//!
//! With the `usage` feature, a [`Metered`] wrapper reports each call's [`Usage`]
//! to a [`UsageSink`] without the recognizer knowing. Collect it per run with a
//! [`UsageCollector`] (one per request, read its total at the end — correct under
//! concurrency), or route it to a process-wide system with [`MetricsSink`]
//! (feature `metrics`) or [`TracingSink`] (feature `metrics-tracing`).

#[doc(inline)]
pub use elide_core::backend::{
    Backend, BackendRequest, BackendResponse, Meter, TokenCounts, Units,
};
/// The `metrics`-facade [`UsageSink`] (Prometheus / StatsD): call, duration,
/// output, and token metrics per model.
#[cfg(feature = "metrics")]
#[cfg_attr(docsrs, doc(cfg(feature = "metrics")))]
#[doc(inline)]
pub use elide_metrics::MetricsSink;
/// Fan a run's usage out to two sinks at once (e.g. a per-run [`UsageCollector`]
/// and a global sink).
#[cfg(feature = "usage")]
#[cfg_attr(docsrs, doc(cfg(feature = "usage")))]
#[doc(inline)]
pub use elide_metrics::Tee;
/// The `tracing` [`UsageSink`]: one event per backend call on the `elide::usage`
/// target.
#[cfg(feature = "metrics-tracing")]
#[cfg_attr(docsrs, doc(cfg(feature = "metrics-tracing")))]
#[doc(inline)]
pub use elide_metrics::TracingSink;
/// The usage machinery: the [`Metered`] wrapper, the [`Usage`] record, and the
/// in-memory [`UsageCollector`].
#[cfg(feature = "usage")]
#[cfg_attr(docsrs, doc(cfg(feature = "usage")))]
#[doc(inline)]
pub use elide_metrics::{Metered, ModelUsage, Outcome, Usage, UsageCollector, UsageSink};
