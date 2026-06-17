//! The [`FusionStrategy`] trait and the shipped strategies.

use veil_core::entity::Entity;
use veil_core::modality::Modality;
use veil_core::primitive::Confidence;

/// How a group of co-located entities' confidences combine into one.
///
/// A *type*, used as the `S` parameter of
/// [`FuseLayer`](super::FuseLayer) — not a stringly-tagged enum. The
/// crate ships the three below; a consumer can implement their own.
/// [`name`](FusionStrategy::name) is recorded in the
/// deduplication [`Event`](veil_core::provenance::Event) recorded on the
/// fused entity.
pub trait FusionStrategy<M: Modality>: Send + Sync {
    /// Stable name of the strategy, recorded in the fusion's `Merge`.
    fn name(&self) -> &'static str;

    /// Combine the group's confidences into the fused confidence.
    ///
    /// `group` is always non-empty.
    fn confidence(&self, group: &[Entity<M>]) -> Confidence;
}

/// `max(p₁, …, pₙ)` — the most confident finding wins.
#[derive(Debug, Clone, Copy, Default)]
pub struct MaxConfidence;

impl<M: Modality> FusionStrategy<M> for MaxConfidence {
    fn name(&self) -> &'static str {
        "max"
    }

    fn confidence(&self, group: &[Entity<M>]) -> Confidence {
        group
            .iter()
            .map(|e| e.confidence)
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(Confidence::MIN)
    }
}

/// Noisy-OR: `1 − ∏(1 − pᵢ)`.
///
/// Treats recognizers as independent witnesses — each adds evidence, so
/// the fused score is monotonic in the number of agreeing detectors and
/// can exceed any single one. Per-recognizer reliability is *not* a
/// fusion concern: scale individual recognizers' scores beforehand with
/// a [`CalibrateLayer`](crate::deduplication::calibrate::CalibrateLayer).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoisyOr;

impl<M: Modality> FusionStrategy<M> for NoisyOr {
    fn name(&self) -> &'static str {
        "noisy_or"
    }

    fn confidence(&self, group: &[Entity<M>]) -> Confidence {
        let product: f64 = group
            .iter()
            .map(|e| 1.0 - f64::from(e.confidence.get()))
            .product();
        Confidence::clamped((1.0 - product) as f32)
    }
}

/// Mean: the arithmetic average of the group's confidences.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mean;

impl<M: Modality> FusionStrategy<M> for Mean {
    fn name(&self) -> &'static str {
        "mean"
    }

    fn confidence(&self, group: &[Entity<M>]) -> Confidence {
        if group.is_empty() {
            return Confidence::MIN;
        }
        let sum: f64 = group.iter().map(|e| f64::from(e.confidence.get())).sum();
        Confidence::clamped((sum / group.len() as f64) as f32)
    }
}
