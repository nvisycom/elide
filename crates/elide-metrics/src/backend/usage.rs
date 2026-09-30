//! The usage record a [`Metered`](crate::Metered) backend call reports, and the
//! sink it reports to.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use elide_core::ErrorKind;
use elide_core::backend::{Meter, TokenCounts, Units};
use elide_core::entity::audit::ModelEvent;
use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// The model a backend called and the billing units the call spent.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ModelUsage {
    /// Model name the backend called (e.g. `"gpt-4o"`, `"gliner-multi"`).
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub model: HipStr<'static>,
    /// Model version, when the backend reports one.
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub version: Option<HipStr<'static>>,
    /// Billing units the call spent (tokens, audio seconds, pages, …), as far as
    /// the provider reports them.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Units::is_empty")
    )]
    pub units: Units,
}

impl ModelUsage {
    /// Model usage naming `model`, with no version and no units yet.
    pub fn new(model: impl Into<HipStr<'static>>) -> Self {
        Self {
            model: model.into(),
            version: None,
            units: Units::none(),
        }
    }

    /// Set the billing units.
    #[must_use]
    pub fn with_units(mut self, units: Units) -> Self {
        self.units = units;
        self
    }
}

impl From<ModelEvent> for ModelUsage {
    /// Take the model identity (name + version) from the audit-trail
    /// [`ModelEvent`] a backend reports, leaving units unset; a backend that
    /// reports units attaches them with [`with_units`](Self::with_units).
    fn from(event: ModelEvent) -> Self {
        Self {
            model: event.name,
            version: event.version,
            units: Units::none(),
        }
    }
}

/// Whether a backend call succeeded, so a sink can track error rate.
///
/// A failed call is recorded like a successful one — with its duration and model
/// — carrying [`Error`](Outcome::Error) and the [`ErrorKind`] it failed with, so
/// latency-on-failure and error rate are visible rather than lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Outcome {
    /// The call returned a response.
    Ok,
    /// The call failed with this [`ErrorKind`].
    Error(ErrorKind),
}

impl Outcome {
    /// A short, stable label for the outcome (`"ok"`, or the error kind's name),
    /// for use as a metric or event dimension.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Error(kind) => kind.as_str(),
        }
    }

    /// Whether the call succeeded.
    #[must_use]
    pub fn is_ok(self) -> bool {
        matches!(self, Outcome::Ok)
    }
}

/// One backend call's resource usage: whether it succeeded, how long it took, the
/// model it called and the units it spent, and how much it produced.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Usage {
    /// Whether the call succeeded or failed.
    pub outcome: Outcome,
    /// Wall-clock time the call took (the model round-trip), measured on both the
    /// success and the failure path.
    #[cfg_attr(feature = "serde", serde(with = "duration_millis"))]
    #[cfg_attr(feature = "schema", schemars(with = "u64"))]
    pub duration: Duration,
    /// The model and unit cost the call incurred. On a failure the units are
    /// whatever the (absent) response reported, i.e. none.
    pub model: ModelUsage,
    /// What the call produced (entities, segments, regions, candidates), when the
    /// response reports a count; `None` on failure or for a response with no
    /// meaningful count.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub output_count: Option<u64>,
}

impl Usage {
    /// A record for a successful call of `duration` against `model` that produced
    /// `output_count`.
    pub fn success(duration: Duration, model: ModelUsage, output_count: Option<u64>) -> Self {
        Self {
            outcome: Outcome::Ok,
            duration,
            model,
            output_count,
        }
    }

    /// A record for a call that failed with `kind` after `duration` against
    /// `model`.
    pub fn failure(duration: Duration, model: ModelUsage, kind: ErrorKind) -> Self {
        Self {
            outcome: Outcome::Error(kind),
            duration,
            model,
            output_count: None,
        }
    }
}

/// Where a [`Metered`](crate::Metered) backend reports each call's [`Usage`].
///
/// The seam: a caller supplies one and hands it to the wrappers at construction.
/// [`UsageCollector`] is the batteries-included in-memory implementation; a caller
/// who wants to route usage to a tracing exporter or a cost model implements this
/// trait instead (see the `TracingSink` / `MetricsSink` sinks).
pub trait UsageSink: Send + Sync {
    /// Record one backend call's usage.
    fn record(&self, usage: Usage);
}

/// An `Arc`'d sink is a sink, so a caller can hold a type-erased
/// `Arc<dyn UsageSink>` (e.g. to combine several concrete sinks behind one type)
/// and still record through it.
impl UsageSink for Arc<dyn UsageSink> {
    fn record(&self, usage: Usage) {
        (**self).record(usage);
    }
}

/// The in-memory [`UsageSink`]: collects every recorded [`Usage`] into a shared
/// `Vec` a caller reads back after a run.
///
/// Cheap to [`Clone`] (it shares one buffer): build one, clone it into each
/// [`Metered`](crate::Metered) wrapper, run the pipeline, then read the entries.
/// Correct under concurrency because each run owns its collector, so a webserver
/// attributes cost to the right request with no shared state.
///
/// ```
/// # use std::sync::Arc;
/// # use elide_metrics::{UsageCollector, UsageSink, Usage, ModelUsage};
/// # use std::time::Duration;
/// let collector = UsageCollector::new();
/// let sink: Arc<dyn UsageSink> = Arc::new(collector.clone());
/// # sink.record(Usage::success(Duration::from_millis(5), ModelUsage::new("gpt-4o"), Some(3)));
/// // ... hand `sink` to `Metered::new(backend, sink)`, run the analysis ...
/// for usage in collector.entries() {
///     println!("{} took {:?}", usage.model.model, usage.duration);
/// }
/// ```
#[derive(Debug, Clone, Default)]
pub struct UsageCollector {
    entries: Arc<Mutex<Vec<Usage>>>,
}

impl UsageCollector {
    /// An empty collector.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A snapshot of every recorded entry, in record order, without clearing.
    ///
    /// Read as often as needed; the collector keeps accumulating.
    #[must_use]
    pub fn entries(&self) -> Vec<Usage> {
        self.entries
            .lock()
            .map(|entries| entries.clone())
            .unwrap_or_default()
    }

    /// Take every recorded entry and clear the collector, so it can be reused for
    /// the next run.
    #[must_use]
    pub fn drain(&self) -> Vec<Usage> {
        self.entries
            .lock()
            .map(|mut entries| std::mem::take(&mut *entries))
            .unwrap_or_default()
    }

    /// The billing units summed across every recorded entry, one total per meter
    /// kind (tokens, audio seconds, pages, …).
    ///
    /// Units are not comparable across models (their prices differ) or across
    /// meter kinds (tokens are not seconds), so this is a raw per-kind sum, not a
    /// cost; attribute per model with [`by_model`](Self::by_model) when several ran.
    #[must_use]
    pub fn total_units(&self) -> Units {
        let mut total = MeterTotals::default();
        for usage in self.entries() {
            for meter in usage.model.units.meters() {
                total.add(*meter);
            }
        }
        total.into_units()
    }

    /// Every entry whose model name matches `model`, in record order, for
    /// attributing usage to one model when several ran.
    #[must_use]
    pub fn by_model(&self, model: &str) -> Vec<Usage> {
        self.entries()
            .into_iter()
            .filter(|usage| usage.model.model == model)
            .collect()
    }

    /// The number of recorded calls, and of those, how many failed.
    ///
    /// `(total, failed)`: `total` is every call the run made through a metered
    /// backend, `failed` the ones that returned an error. `total - failed`
    /// succeeded.
    #[must_use]
    pub fn call_counts(&self) -> (usize, usize) {
        let entries = self.entries();
        let failed = entries.iter().filter(|u| !u.outcome.is_ok()).count();
        (entries.len(), failed)
    }

    /// The total output the run produced — entities, segments, regions,
    /// candidates — summed across every entry that reported a count.
    #[must_use]
    pub fn output_total(&self) -> u64 {
        self.entries()
            .iter()
            .filter_map(|usage| usage.output_count)
            .sum()
    }
}

impl UsageSink for UsageCollector {
    fn record(&self, usage: Usage) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.push(usage);
        }
    }
}

/// Running per-kind totals while summing [`Meter`]s across entries, folded back
/// into a [`Units`] carrying one meter per kind that was seen.
#[derive(Default)]
struct MeterTotals {
    tokens: Option<TokenCounts>,
    seconds: Option<f64>,
    pages: Option<u64>,
    images: Option<u64>,
    characters: Option<u64>,
}

impl MeterTotals {
    /// Fold one meter into the running total for its kind.
    fn add(&mut self, meter: Meter) {
        let sum = |acc: &mut Option<u64>, value: u64| *acc = Some(acc.unwrap_or(0) + value);
        match meter {
            Meter::Tokens(counts) => {
                let acc = self.tokens.get_or_insert_with(TokenCounts::default);
                let field = |a: &mut Option<u64>, b: Option<u64>| {
                    if let Some(b) = b {
                        *a = Some(a.unwrap_or(0) + b);
                    }
                };
                field(&mut acc.input, counts.input);
                field(&mut acc.output, counts.output);
                field(&mut acc.total, counts.total);
                field(&mut acc.cached, counts.cached);
                field(&mut acc.reasoning, counts.reasoning);
            }
            Meter::Seconds(seconds) => {
                self.seconds = Some(self.seconds.unwrap_or(0.0) + seconds);
            }
            Meter::Pages(pages) => sum(&mut self.pages, pages),
            Meter::Images(images) => sum(&mut self.images, images),
            Meter::Characters(characters) => sum(&mut self.characters, characters),
        }
    }

    /// Collapse the running totals into a [`Units`] with one meter per kind seen.
    fn into_units(self) -> Units {
        let mut units = Units::none();
        if let Some(tokens) = self.tokens {
            units = units.with(Meter::Tokens(tokens));
        }
        if let Some(seconds) = self.seconds {
            units = units.with(Meter::Seconds(seconds));
        }
        if let Some(pages) = self.pages {
            units = units.with(Meter::Pages(pages));
        }
        if let Some(images) = self.images {
            units = units.with(Meter::Images(images));
        }
        if let Some(characters) = self.characters {
            units = units.with(Meter::Characters(characters));
        }
        units
    }
}

/// Serialize a [`Duration`] as whole milliseconds, keeping the wire form a plain
/// integer rather than serde's default `{ secs, nanos }` object.
#[cfg(feature = "serde")]
mod duration_millis {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_millis(u64::deserialize(d)?))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use elide_core::backend::{Meter, TokenCounts, Units};

    use super::{ModelUsage, Usage, UsageCollector, UsageSink};

    fn record(collector: &UsageCollector, model: &str, units: Units, output: Option<u64>) {
        let model = ModelUsage::new(model).with_units(units);
        collector.record(Usage::success(Duration::from_millis(1), model, output));
    }

    #[test]
    fn total_units_sums_each_meter_kind_across_entries() {
        let collector = UsageCollector::new();
        let tokens = |input, output| {
            Units::from(Meter::Tokens(TokenCounts {
                input: Some(input),
                output: Some(output),
                ..TokenCounts::default()
            }))
        };
        record(&collector, "gpt-4o", tokens(10, 5), Some(3));
        record(&collector, "gpt-4o", tokens(4, 1), Some(2));
        record(
            &collector,
            "whisper",
            Units::from(Meter::Seconds(12.5)),
            Some(7),
        );
        record(
            &collector,
            "whisper",
            Units::from(Meter::Seconds(2.5)),
            Some(1),
        );
        record(&collector, "ocr", Units::from(Meter::Images(1)), None);

        let total = collector.total_units();
        let meters = total.meters();

        assert!(meters.contains(&Meter::Tokens(TokenCounts {
            input: Some(14),
            output: Some(6),
            ..TokenCounts::default()
        })));
        assert!(meters.contains(&Meter::Seconds(15.0)));
        assert!(meters.contains(&Meter::Images(1)));
    }

    #[test]
    fn total_units_is_empty_when_no_backend_bills() {
        let collector = UsageCollector::new();
        record(&collector, "gliner", Units::none(), Some(4));
        assert!(collector.total_units().is_empty());
    }

    #[test]
    fn call_and_output_totals_track_every_entry() {
        let collector = UsageCollector::new();
        record(&collector, "gpt-4o", Units::none(), Some(3));
        record(&collector, "gpt-4o", Units::none(), Some(2));
        collector.record(Usage::failure(
            Duration::from_millis(1),
            ModelUsage::new("gpt-4o"),
            elide_core::ErrorKind::Provider,
        ));

        assert_eq!(collector.call_counts(), (3, 1));
        assert_eq!(collector.output_total(), 5);
    }
}
