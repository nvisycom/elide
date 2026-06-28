//! [`AggregationStrategy`]: how per-token NER predictions are collapsed
//! into entity spans.

#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How adjacent token predictions merge into one span.
///
/// Names follow HuggingFace
/// `transformers.pipeline(aggregation_strategy=...)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum AggregationStrategy {
    /// No aggregation: every token-level prediction becomes its own
    /// span. Cheapest, noisiest.
    None,
    /// Merge adjacent tokens that share a label. The span's score
    /// is the first token's. Useful when later tokens of a span
    /// continue the same entity (BIO-style `B-PER` then `I-PER`).
    First,
    /// Merge adjacent same-label tokens; the span's score is the
    /// maximum across constituent tokens. The default: works best
    /// when models output per-token confidences without a strong
    /// boundary signal.
    #[default]
    Max,
    /// Merge adjacent same-label tokens; the span's score is the
    /// arithmetic mean across constituent tokens. Smoother than
    /// [`Max`], more lenient at span boundaries.
    ///
    /// [`Max`]: Self::Max
    Average,
    /// Merge based on `B-`/`I-`/`O` BIO tags rather than label
    /// equality. The span's score is the first token's. Match
    /// HuggingFace `"simple"` strategy.
    Simple,
}
