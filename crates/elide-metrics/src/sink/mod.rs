//! Sinks that route backend [`Usage`] to process-wide
//! observability systems, and the [`Tee`] combinator that feeds two at once.
//!
//! - `TracingSink` (feature `tracing`) emits a `tracing` event per call.
//! - `MetricsSink` (feature `metrics`) records call/duration/output/token
//!   metrics through the `metrics` facade.

#[cfg(feature = "metrics")]
mod metrics;
#[cfg(feature = "tracing")]
mod tracing;

#[cfg(feature = "metrics")]
#[cfg_attr(docsrs, doc(cfg(feature = "metrics")))]
pub use self::metrics::MetricsSink;
#[cfg(feature = "tracing")]
#[cfg_attr(docsrs, doc(cfg(feature = "tracing")))]
pub use self::tracing::TracingSink;
use crate::backend::{Usage, UsageSink};

/// A [`UsageSink`] that forwards each record to two sinks.
///
/// For the common case where a run's usage should reach both a per-run
/// [`UsageCollector`](crate::backend::UsageCollector) (for that request's total)
/// and a global sink like `TracingSink` (for the dashboard) at once. Compose
/// more than two by nesting: `Tee::new(a, Tee::new(b, c))`.
pub struct Tee<A, B> {
    first: A,
    second: B,
}

impl<A, B> Tee<A, B> {
    /// Forward each record to `first` then `second`.
    pub fn new(first: A, second: B) -> Self {
        Self { first, second }
    }
}

impl<A: UsageSink, B: UsageSink> UsageSink for Tee<A, B> {
    fn record(&self, usage: Usage) {
        self.first.record(usage.clone());
        self.second.record(usage);
    }
}
