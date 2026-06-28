//! The [`Scoring`] trait — how merged confidence is scored — and its impls.

use elide_core::modality::Modality;
use elide_core::primitive::Confidence;

/// How two grouped confidences combine into the merged finding's score.
///
/// A *type*, the `S` parameter of [`Merging`]. Some scorings *pick* an existing
/// score ([`MaxConfidence`]), others *compute* a new one ([`NoisyOrConfidence`]);
/// the trait abstracts over both. The crate ships [`MaxConfidence`] (the
/// default) and [`NoisyOrConfidence`]; a consumer can implement their own.
/// Scoring is pairwise — the [`Merging`] reconciler applies it to each merged
/// pair — so a scoring must be associative for a cluster of three or more to
/// combine consistently (both shipped scorings are).
///
/// [`Merging`]: super::Merging
pub trait Scoring<M: Modality>: Send + Sync {
    /// Stable name of the scoring, recorded in the fusion event.
    fn name(&self) -> &'static str;

    /// Combine two confidences into the merged score.
    fn score(&self, a: Confidence, b: Confidence) -> Confidence;
}

/// `max(a, b)` — the more confident finding wins.
///
/// The conservative default: corroboration never *lowers* the score, and a
/// single strong witness carries the cluster. The merged confidence is one
/// recognizer's existing score, so it assumes nothing about the members being
/// independent or their scores being comparable probabilities. Associative.
#[derive(Debug, Clone, Copy, Default)]
pub struct MaxConfidence;

impl<M: Modality> Scoring<M> for MaxConfidence {
    fn name(&self) -> &'static str {
        "max"
    }

    fn score(&self, a: Confidence, b: Confidence) -> Confidence {
        if a >= b { a } else { b }
    }
}

/// Noisy-OR of two scores: `1 − (1 − a)(1 − b)`.
///
/// Treats recognizers as independent witnesses — each adds evidence, so the
/// fused score is monotonic in the number of agreeing detectors and can exceed
/// any single one. Sound *only* when the members really are independent and
/// their scores are calibrated probabilities; correlated detectors (two regexes
/// for the same pattern) inflate the score. Per-recognizer reliability is *not*
/// a fusion concern — scale individual recognizers' scores beforehand with a
/// [`CalibrateLayer`]. Associative, so it composes consistently across a cluster
/// of three or more.
///
/// [`CalibrateLayer`]: crate::layer::calibrate::CalibrateLayer
#[derive(Debug, Clone, Copy, Default)]
pub struct NoisyOrConfidence;

impl<M: Modality> Scoring<M> for NoisyOrConfidence {
    fn name(&self) -> &'static str {
        "noisy_or"
    }

    fn score(&self, a: Confidence, b: Confidence) -> Confidence {
        let combined = 1.0 - (1.0 - f64::from(a.get())) * (1.0 - f64::from(b.get()));
        Confidence::clamped(combined as f32)
    }
}
