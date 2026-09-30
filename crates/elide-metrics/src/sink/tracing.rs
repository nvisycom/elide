//! [`TracingSink`]: forward backend usage to `tracing`.

use crate::backend::{Usage, UsageSink};

/// Emits each backend call's [`Usage`] as a `tracing` event on the
/// `elide::usage` target, so it lands in whatever subscriber the process runs
/// alongside the backends' own instrumentation spans.
///
/// The event carries the model name and version, the call duration in
/// milliseconds, and each billing meter the call reported (tokens, audio seconds,
/// pages, …). `tracing` fields are statically named, so the meter kinds map to
/// fixed fields, each `None` when the call did not report it. A subscriber filters
/// or aggregates it like any other event; nothing is collected in process here.
#[derive(Debug, Clone, Copy, Default)]
pub struct TracingSink;

impl TracingSink {
    /// A tracing sink.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl UsageSink for TracingSink {
    fn record(&self, usage: Usage) {
        let units = &usage.model.units;
        let tokens = units.tokens().unwrap_or_default();

        tracing::info!(
            target: "elide::usage",
            outcome = usage.outcome.label(),
            model = %usage.model.model,
            version = usage.model.version.as_deref(),
            duration_ms = u64::try_from(usage.duration.as_millis()).unwrap_or(u64::MAX),
            output = usage.output_count,
            input_tokens = tokens.input,
            output_tokens = tokens.output,
            total_tokens = tokens.total,
            seconds = units.seconds(),
            pages = units.pages(),
            images = units.images(),
            characters = units.characters(),
            "backend call",
        );
    }
}
