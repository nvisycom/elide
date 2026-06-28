//! The [`Merging`] reconciler.

use elide_core::entity::Entity;
use elide_core::modality::Modality;

use super::{Disposition, Reconciler};
use crate::layer::reconcile::scoring::{Max, Strategy};

/// The merging reconciler: every grouped pair merges, scoring the combined
/// confidence with a [`Strategy`].
///
/// The fusion behavior — combine co-located same-label findings into one
/// entity. Generic over the scoring strategy `S`, chosen at construction.
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
