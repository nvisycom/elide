//! Resource-usage accounting for one detection run.
//!
//! Every recognizer and enricher a payload passes through contributes a
//! [`Usage`]: its identity, how long it ran, how much it found, and, for a
//! model-backed component, the model it called and the tokens that cost.
//! The analyzer measures the always-present facts (time, count) at the call
//! boundary; the component itself supplies the model detail it alone can see
//! (via [`Recognition`]/[`Enrichment`]). Per-document, the entries aggregate
//! into a [`UsageReport`].
//!
//! [`Recognition`]: crate::recognition::Recognition
//! [`Enrichment`]: crate::enrichment::Enrichment

use std::time::Duration;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::entity::audit::ModelEvent;
use crate::recognition::RecognizerId;

/// Serialize a [`Duration`] as a whole number of milliseconds (and read it
/// back), keeping the wire form a plain integer rather than serde's default
/// `{ secs, nanos }` object.
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

/// The token counts a model reported.
///
/// Each is optional because providers differ in what they return: some give
/// only a total, some none at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct TokenCounts {
    /// Prompt / input tokens.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub input: Option<u64>,
    /// Completion / output tokens.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub output: Option<u64>,
    /// Total tokens, may be reported even when the input/output split is not.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub total: Option<u64>,
}

impl TokenCounts {
    /// Whether any count is present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.input.is_none() && self.output.is_none() && self.total.is_none()
    }
}

/// The model a model-backed recognizer or enricher called, and its token cost.
///
/// Absent from a pure-CPU component (a pattern recognizer, a language enricher).
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Tokens the call spent, as far as the provider reports them.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "TokenCounts::is_empty")
    )]
    pub tokens: TokenCounts,
}

impl ModelUsage {
    /// Model usage naming `model`, with no version and no tokens yet.
    pub fn new(model: impl Into<HipStr<'static>>) -> Self {
        Self {
            model: model.into(),
            version: None,
            tokens: TokenCounts::default(),
        }
    }

    /// Set the model version.
    #[must_use]
    pub fn with_version(mut self, version: impl Into<HipStr<'static>>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Set the token counts.
    #[must_use]
    pub fn with_tokens(mut self, tokens: TokenCounts) -> Self {
        self.tokens = tokens;
        self
    }
}

impl From<ModelEvent> for ModelUsage {
    /// Take a model's identity (name + version) from the audit-trail
    /// [`ModelEvent`] a backend reports via its `provenance()`, dropping the
    /// per-entity `contextual` flag and leaving tokens unset; a backend that
    /// reports tokens attaches them with [`with_tokens`](Self::with_tokens).
    fn from(event: ModelEvent) -> Self {
        Self {
            model: event.name,
            version: event.version,
            tokens: TokenCounts::default(),
        }
    }
}

/// One recognizer's or enricher's resource usage for one payload.
///
/// `id` and `duration` are always present; `count` (entities or spans found,
/// artifacts produced) and `model` are present only where they mean
/// something. `duration` serializes as a whole number of milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Usage {
    /// Which recognizer / enricher this describes.
    pub id: RecognizerId,
    /// Wall-clock execution time. Serialized as integer milliseconds.
    #[cfg_attr(feature = "serde", serde(with = "duration_millis"))]
    #[cfg_attr(feature = "schema", schemars(with = "u64"))]
    pub duration: Duration,
    /// Entities or spans found (recognizer) or artifacts produced (enricher);
    /// `None` when a count is not meaningful for the component.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub count: Option<u64>,
    /// Model / token detail; `None` for a pure-CPU component.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub model: Option<ModelUsage>,
}

impl Usage {
    /// A record with a duration and a count, and no model detail, the shape a
    /// pure-CPU recognizer produces.
    pub fn new(id: RecognizerId, duration: Duration, count: u64) -> Self {
        Self {
            id,
            duration,
            count: Some(count),
            model: None,
        }
    }

    /// A record with a duration and no count, the shape an enricher produces
    /// (an enricher yields context, not counted entities).
    pub fn timed(id: RecognizerId, duration: Duration) -> Self {
        Self {
            id,
            duration,
            count: None,
            model: None,
        }
    }

    /// Attach model / token detail (the model-backed path).
    #[must_use]
    pub fn with_model(mut self, model: ModelUsage) -> Self {
        self.model = Some(model);
        self
    }

    /// Set the count (entities/spans/artifacts).
    #[must_use]
    pub fn with_count(mut self, count: u64) -> Self {
        self.count = Some(count);
        self
    }
}

/// Every recognizer/enricher's [`Usage`] across a whole document analysis, in
/// the order the components ran (the body first, then each part).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct UsageReport {
    /// The per-component usage entries, each self-identifying via its `id`.
    pub entries: Vec<Usage>,
}

impl UsageReport {
    /// An empty report.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether any usage was recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every entry recorded for the component named `name` (a recognizer's or
    /// enricher's [`RecognizerId::name`]), in run order.
    ///
    /// The pure-CPU singletons report a fixed name, `"elide-pattern"`,
    /// `"elide-lingua"`, but each model-backed component (NER, LLM, OCR, STT)
    /// reports the name its *caller* configured, so match those against the
    /// name you built the component with. Token cost is left on each entry
    /// deliberately: tokens are not comparable across models (their prices
    /// differ), so this returns the raw entries rather than any summed figure.
    ///
    /// [`RecognizerId::name`]: crate::recognition::RecognizerId::name
    pub fn by_name<'a>(&'a self, name: &str) -> impl Iterator<Item = &'a Usage> {
        let name = name.to_owned();
        self.entries.iter().filter(move |u| u.id.name == name)
    }

    /// Append more usage entries.
    pub fn extend(&mut self, more: impl IntoIterator<Item = Usage>) {
        self.entries.extend(more);
    }
}

#[cfg(test)]
mod report_tests {
    use super::*;

    #[test]
    fn by_name_filters_to_one_component() {
        let mut report = UsageReport::new();
        report.extend([
            Usage::new(RecognizerId::new("elide-pattern", "1"), Duration::ZERO, 2),
            Usage::new(RecognizerId::new("acme-ner", "1"), Duration::ZERO, 1),
            Usage::new(RecognizerId::new("elide-pattern", "1"), Duration::ZERO, 3),
        ]);
        let counts: Vec<_> = report.by_name("elide-pattern").map(|u| u.count).collect();
        assert_eq!(counts, [Some(2), Some(3)]);
        assert_eq!(report.by_name("acme-ner").count(), 1);
        assert_eq!(report.by_name("absent").count(), 0);
    }
}

// Only the custom serde behavior is worth testing here: the hand-rolled
// millisecond `Duration` codec and the `skip_serializing_if` omissions.
#[cfg(all(test, feature = "serde"))]
mod tests {
    use super::*;

    fn id() -> RecognizerId {
        RecognizerId::new("elide-pattern", "1.2.3")
    }

    #[test]
    fn serde_duration_is_millis_and_omits_absent_optionals() {
        let u = Usage::new(id(), Duration::from_millis(7), 2);
        let v = serde_json::to_value(&u).expect("serialize");
        // Our `duration_millis` codec emits a plain integer, not `{secs,nanos}`.
        assert_eq!(v["duration"], 7);
        assert_eq!(v["count"], 2);
        // An absent model is omitted entirely, not serialized as `null`.
        assert!(v.get("model").is_none(), "model should be omitted: {v}");
    }

    #[test]
    fn serde_round_trips_with_model() {
        let u = Usage::new(id(), Duration::from_millis(50), 1).with_model(
            ModelUsage::new("mock").with_tokens(TokenCounts {
                total: Some(9),
                ..TokenCounts::default()
            }),
        );
        let json = serde_json::to_string(&u).expect("serialize");
        // The millisecond codec must also read back (deserialize) correctly.
        let back: Usage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(u, back);
    }
}
