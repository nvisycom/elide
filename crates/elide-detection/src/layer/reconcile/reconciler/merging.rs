//! The [`Merging`] reconciler.

use elide_core::entity::Entity;
use elide_core::modality::Modality;

use super::{Disposition, Reconciler};
use crate::layer::reconcile::scoring::{Max, NoisyOr, Strategy};

/// The merging reconciler: combine every grouped pair into one entity.
///
/// The fusion behavior — co-located same-label findings merge over the union
/// of their spans, with confidence pooled by a [`Strategy`]. Generic over the
/// scoring strategy `S`, chosen at construction.
///
/// [`Strategy`]: crate::layer::reconcile::scoring::Strategy
#[derive(Debug, Clone, Copy, Default)]
pub struct Merging<S = Max> {
    /// How the pair's confidences combine into the merged score.
    pub strategy: S,
}

impl<S> Merging<S> {
    /// A merging reconciler scoring with `strategy`.
    pub fn new(strategy: S) -> Self {
        Self { strategy }
    }
}

impl Merging<Max> {
    /// A merging reconciler scoring by [`Max`] — the most confident finding
    /// wins (the default).
    pub fn max() -> Self {
        Self::new(Max)
    }
}

impl Merging<NoisyOr> {
    /// A merging reconciler scoring by [`NoisyOr`] — agreeing detectors
    /// accumulate evidence (`1 − ∏(1 − pᵢ)`).
    pub fn noisy_or() -> Self {
        Self::new(NoisyOr)
    }
}

impl<M, S> Reconciler<M> for Merging<S>
where
    M: Modality,
    S: Strategy<M>,
{
    fn decide(&self, a: &Entity<M>, b: &Entity<M>) -> Disposition {
        Disposition::Merge {
            confidence: self.strategy.score(a.confidence, b.confidence),
        }
    }

    fn name(&self) -> &'static str {
        self.strategy.name()
    }
}
