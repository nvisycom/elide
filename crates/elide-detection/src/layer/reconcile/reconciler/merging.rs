//! The [`Merging`] reconciler.

use elide_core::entity::Entity;
use elide_core::modality::Modality;

use super::{Disposition, Reconciler};
use crate::layer::reconcile::scoring::{MaxConfidence, NoisyOrConfidence, Scoring};

/// The merging reconciler: combine every grouped pair into one entity.
///
/// The fusion behavior — co-located same-label findings merge over the union
/// of their spans, with confidence combined by a [`Scoring`]. Generic over the
/// scoring `S`, chosen at construction.
///
/// [`Scoring`]: crate::layer::reconcile::scoring::Scoring
#[derive(Debug, Clone, Copy, Default)]
pub struct Merging<S = MaxConfidence> {
    /// How the pair's confidences combine into the merged score.
    pub scoring: S,
}

impl<S> Merging<S> {
    /// A merging reconciler combining confidences with `scoring`.
    pub fn new(scoring: S) -> Self {
        Self { scoring }
    }
}

impl Merging<MaxConfidence> {
    /// A merging reconciler scoring by [`MaxConfidence`] — the most confident
    /// finding wins (the default).
    pub fn max() -> Self {
        Self::new(MaxConfidence)
    }
}

impl Merging<NoisyOrConfidence> {
    /// A merging reconciler scoring by [`NoisyOrConfidence`] — agreeing
    /// detectors accumulate evidence (`1 − ∏(1 − pᵢ)`).
    pub fn noisy_or() -> Self {
        Self::new(NoisyOrConfidence)
    }
}

impl<M, S> Reconciler<M> for Merging<S>
where
    M: Modality,
    S: Scoring<M>,
{
    fn decide(&self, a: &Entity<M>, b: &Entity<M>) -> Disposition {
        Disposition::Merge {
            confidence: self.scoring.score(a.confidence, b.confidence),
        }
    }

    fn name(&self) -> &'static str {
        self.scoring.name()
    }
}
