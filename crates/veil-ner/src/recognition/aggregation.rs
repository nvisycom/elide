//! [`AggregationStrategy`] and [`AlignmentMode`]: policies for
//! collapsing per-token NER predictions into entity spans.
//!
//! The producer engine may apply them server-side (the Bento
//! `inference-gliner` already returns aggregated spans), in which
//! case the consumer-side knobs are advisory; or the producer may
//! emit unaggregated token-classification output, in which case
//! [`NerRecognizer`] applies them itself.
//!
//! [`NerRecognizer`]: super::NerRecognizer

use serde::{Deserialize, Serialize};

/// How adjacent token-level predictions of the same entity type are
/// merged into one span. Names follow HuggingFace
/// `transformers.pipeline(aggregation_strategy=...)` semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
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
    /// maximum across constituent tokens. The default — works best
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

/// How a token-classification span snaps to character boundaries
/// when the model emits sub-word predictions. Mirrors spaCy
/// `Doc.char_span(alignment_mode=...)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlignmentMode {
    /// Reject spans that don't land on a token boundary.
    Strict,
    /// Shrink the span to the next inner token boundary.
    Contract,
    /// Expand the span to the next outer token boundary. Default —
    /// favors recall over precision.
    #[default]
    Expand,
}
