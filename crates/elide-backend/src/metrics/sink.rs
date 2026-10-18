//! [`Tee`]: a [`UsageSink`] combinator that feeds two sinks at once.
//!
//! The sinks that route usage to a process-wide system —
//! [`TracingSink`](super::TracingSink) (feature `tracing`) and
//! [`MetricsSink`](super::MetricsSink) (feature `recorder`) — are siblings in the
//! parent module.

use super::{Usage, UsageSink};

/// A [`UsageSink`] that forwards each record to two sinks.
///
/// For the common case where a run's usage should reach both a per-run
/// [`UsageCollector`](super::UsageCollector) (for that request's total) and a
/// global sink like `TracingSink` (for the dashboard) at once. Compose more than
/// two by nesting: `Tee::new(a, Tee::new(b, c))`.
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
