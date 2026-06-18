//! [`NerModel`]: client-side NER tuning knobs.
//!
//! Applied inside [`NerRecognizer`] before entities are emitted,
//! so backends stay dumb and label normalization is uniform
//! across them.
//!
//! Construct via [`NerModel::default`] for the canonical defaults
//! (canonical label map, no ignored labels, score = 0.85, no
//! demotion), sensible defaults for a backend that already returns
//! aggregated, canonically-labeled spans, or via [`NerModel::builder`]
//! for an infallible chainable builder that overrides only the fields
//! you care about.
//!
//! [`NerRecognizer`]: super::NerRecognizer

use std::collections::HashSet;

use derive_builder::Builder;
use elide_core::primitive::Confidence;
use elide_core::recognition::LabelMap;

use super::aggregation::{AggregationStrategy, AlignmentMode};

/// Per-recognizer NER policy.
#[derive(Debug, Clone, Builder)]
#[builder(
    name = "NerModelBuilder",
    pattern = "owned",
    setter(into, prefix = "with"),
    build_fn(skip)
)]
pub struct NerModel {
    /// Translation from raw model labels to canonical entity
    /// label names. Defaults to [`LabelMap::canonical`].
    pub label_map: LabelMap,
    /// Raw labels the adapter drops without translation. Useful
    /// for filtering out labels the model emits but we don't care
    /// about (e.g. `O` from BIO tagging, `MISC` from generic NER
    /// schemas).
    pub labels_to_ignore: HashSet<String>,
    /// Fallback confidence when a backend emits a score outside
    /// `[0.0, 1.0]` (treated as a bug; clamped + this used as the
    /// safe default).
    pub default_score: Confidence,
    /// Entity label names whose emitted confidence is multiplied
    /// by `low_score_multiplier` before being surfaced. Use for
    /// noisy-but-high-recall labels.
    pub low_score_labels: HashSet<String>,
    /// Multiplier applied to `low_score_labels`. Must be in
    /// `[0.0, 1.0]`.
    pub low_score_multiplier: f64,
    /// Aggregation policy for backends that emit token-level
    /// predictions. Advisory for bac kends that aggregate
    /// server-side.
    pub aggregation: AggregationStrategy,
    /// Alignment policy for sub-word predictions. Same advisory
    /// status as `aggregation`.
    pub alignment: AlignmentMode,
}

impl Default for NerModel {
    fn default() -> Self {
        Self {
            label_map: LabelMap::canonical(),
            labels_to_ignore: HashSet::new(),
            default_score: Confidence::new(0.85).expect("0.85 in range"),
            low_score_labels: HashSet::new(),
            low_score_multiplier: 0.4,
            aggregation: AggregationStrategy::Max,
            alignment: AlignmentMode::Expand,
        }
    }
}

impl NerModel {
    /// Start a chainable, infallible builder seeded from the
    /// canonical defaults; every field has a sensible default, so
    /// callers only override what they care about.
    #[must_use]
    pub fn builder() -> NerModelBuilder {
        NerModelBuilder::default()
    }
}

impl NerModelBuilder {
    /// Finish the builder, filling every unset field with its
    /// default. Infallible: no required fields.
    #[must_use]
    pub fn build(self) -> NerModel {
        let defaults = NerModel::default();
        NerModel {
            label_map: self.label_map.unwrap_or(defaults.label_map),
            labels_to_ignore: self.labels_to_ignore.unwrap_or(defaults.labels_to_ignore),
            default_score: self.default_score.unwrap_or(defaults.default_score),
            low_score_labels: self.low_score_labels.unwrap_or(defaults.low_score_labels),
            low_score_multiplier: self
                .low_score_multiplier
                .unwrap_or(defaults.low_score_multiplier),
            aggregation: self.aggregation.unwrap_or(defaults.aggregation),
            alignment: self.alignment.unwrap_or(defaults.alignment),
        }
    }
}
