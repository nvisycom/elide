//! [`MetricsSink`]: forward backend usage to the `metrics` facade.

use metrics::{Unit, counter, describe_counter, describe_histogram, histogram};

use crate::backend::{Usage, UsageSink};

/// Backend calls made (labelled by outcome), for rate and error rate.
const CALLS: &str = "elide.backend.calls";
/// Call wall-clock time, in milliseconds.
const DURATION: &str = "elide.backend.duration";
/// Output the call produced (entities / segments / regions / candidates).
const OUTPUT: &str = "elide.backend.output";
/// Prompt / input tokens.
const INPUT_TOKENS: &str = "elide.backend.tokens.input";
/// Completion / output tokens.
const OUTPUT_TOKENS: &str = "elide.backend.tokens.output";
/// Total tokens the provider reported.
const TOTAL_TOKENS: &str = "elide.backend.tokens.total";
/// Seconds of media processed.
const SECONDS: &str = "elide.backend.seconds";
/// Pages processed.
const PAGES: &str = "elide.backend.pages";
/// Images processed.
const IMAGES: &str = "elide.backend.images";
/// Characters processed.
const CHARACTERS: &str = "elide.backend.characters";

/// Records each backend call's [`Usage`] through the `metrics` facade: a call
/// duration histogram and one counter per billing meter the call reported
/// (tokens, audio seconds, pages, …), each labelled by the `model` and `version`
/// that ran.
///
/// For a process-wide aggregate view — total tokens, latency percentiles — via a
/// recorder the process installs (Prometheus, StatsD, …). This is *global* by
/// design; for per-run cost (a single request's total), use a per-run
/// [`UsageCollector`](crate::backend::UsageCollector) instead, or a
/// [`Tee`](crate::sink::Tee) that feeds both.
///
/// Metric names are dotted and unit-free; the recorder's exporter maps them to
/// its own convention (Prometheus turns `.` into `_`), and the units are declared
/// through [`describe`](Self::describe). The `model`/`version` labels are a
/// bounded set (the models a deployment configures), so they are safe as labels;
/// per-request identifiers are deliberately not labelled here, as they would
/// explode metric cardinality.
#[derive(Debug, Clone, Copy, Default)]
pub struct MetricsSink;

impl MetricsSink {
    /// A metrics sink.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Register the metrics' units and help text with the installed recorder.
    ///
    /// Call once after installing the recorder and before the first backend call,
    /// so exporters that surface unit and description metadata (Prometheus `HELP`
    /// / `TYPE`, OpenTelemetry) have it. Recording works without it; this only
    /// enriches the output.
    pub fn describe() {
        describe_counter!(CALLS, Unit::Count, "Backend calls made, by outcome");
        describe_histogram!(DURATION, Unit::Milliseconds, "Backend call wall-clock time");
        describe_counter!(OUTPUT, Unit::Count, "Items a backend call produced");
        describe_counter!(
            INPUT_TOKENS,
            Unit::Count,
            "Prompt tokens a backend call reported"
        );
        describe_counter!(
            OUTPUT_TOKENS,
            Unit::Count,
            "Completion tokens a backend call reported"
        );
        describe_counter!(
            TOTAL_TOKENS,
            Unit::Count,
            "Total tokens a backend call reported"
        );
        describe_histogram!(
            SECONDS,
            Unit::Seconds,
            "Seconds of media a backend call processed"
        );
        describe_counter!(PAGES, Unit::Count, "Pages a backend call processed");
        describe_counter!(IMAGES, Unit::Count, "Images a backend call processed");
        describe_counter!(
            CHARACTERS,
            Unit::Count,
            "Characters a backend call processed"
        );
    }
}

impl UsageSink for MetricsSink {
    fn record(&self, usage: Usage) {
        let model = usage.model.model.to_string();
        let version = usage
            .model
            .version
            .map_or_else(String::new, |v| v.to_string());
        let outcome = usage.outcome.label();
        let ms = usage.duration.as_secs_f64() * 1000.0;

        // One call, labelled by outcome, so error rate is a ratio over this counter.
        counter!(CALLS, "model" => model.clone(), "version" => version.clone(), "outcome" => outcome)
            .increment(1);
        // Latency is measured on both paths, labelled so failures can be excluded.
        histogram!(DURATION, "model" => model.clone(), "version" => version.clone(), "outcome" => outcome)
            .record(ms);

        if let Some(output) = usage.output_count {
            counter!(OUTPUT, "model" => model.clone(), "version" => version.clone())
                .increment(output);
        }

        let units = &usage.model.units;
        let count = |name: &'static str, value: Option<u64>| {
            if let Some(value) = value {
                counter!(name, "model" => model.clone(), "version" => version.clone())
                    .increment(value);
            }
        };
        let tokens = units.tokens().unwrap_or_default();
        count(INPUT_TOKENS, tokens.input);
        count(OUTPUT_TOKENS, tokens.output);
        count(TOTAL_TOKENS, tokens.total);
        count(PAGES, units.pages());
        count(IMAGES, units.images());
        count(CHARACTERS, units.characters());
        // A histogram so fractional seconds survive; a counter forces whole-second
        // rounding.
        if let Some(seconds) = units.seconds() {
            histogram!(SECONDS, "model" => model.clone(), "version" => version.clone())
                .record(seconds);
        }
    }
}
