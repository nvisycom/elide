//! The [`Layer`] pipeline: post-detection stages that reshape and prune the
//! working entity set.
//!
//! Recognizers emit entities independently; these [`Layer`]s reshape and prune
//! them. They are composed onto an [`Analyzer`], which runs them in order after
//! detection. The shipped stages, in their usual order:
//!
//! 1. [`calibrate`], scale each entity's confidence by a per-recognizer
//!    multiplier, so detectors with different score distributions are
//!    comparable before reconciliation.
//! 2. [`overlap`], decide what happens to overlapping entities: cluster them
//!    ([`GroupPredicate`]), fuse each cluster of the same finding
//!    ([`FuseLayer`]), and arbitrate cross-label pairs ([`Reconciler`]):
//!    keep, contest, or resolve.
//! 3. [`filter`], drop entities below a confidence threshold.
//!
//! Each stage implements [`Layer`], returning a [`LayerOutput`]. Stages are
//! pure and synchronous, reached through their submodules (e.g.
//! [`overlap::ReconcileLayer`]).
//!
//! [`Analyzer`]: crate::Analyzer
//! [`GroupPredicate`]: overlap::group::GroupPredicate
//! [`FuseLayer`]: overlap::FuseLayer
//! [`Reconciler`]: overlap::Reconciler

pub mod calibrate;
pub mod filter;
pub mod overlap;

use elide_core::entity::Entity;
use elide_core::modality::Modality;

/// The result of running one [`Layer`]: the entities kept and those dropped.
///
/// Names the two outcomes explicitly so call sites don't juggle a positional
/// tuple. The pipeline threads [`kept`] into the next layer and accumulates
/// [`dropped`] for logging.
///
/// [`kept`]: Self::kept
/// [`dropped`]: Self::dropped
#[derive(Debug)]
pub struct LayerOutput<M: Modality> {
    /// Entities that survived this layer.
    pub kept: Vec<Entity<M>>,
    /// Entities this layer removed.
    pub dropped: Vec<Entity<M>>,
}

impl<M: Modality> LayerOutput<M> {
    /// All entities kept, none dropped, for a layer that only reshapes them.
    pub fn kept(entities: Vec<Entity<M>>) -> Self {
        Self {
            kept: entities,
            dropped: Vec::new(),
        }
    }

    /// A kept/dropped split.
    pub fn split(kept: Vec<Entity<M>>, dropped: Vec<Entity<M>>) -> Self {
        Self { kept, dropped }
    }
}

/// One stage of the layer pipeline: takes entities, keeps or drops them.
///
/// A layer takes the working set of entities and returns the reshaped set plus
/// anything it removed. Stages are **pure and synchronous**, they operate over
/// a `Vec<Entity<M>>` in memory with no I/O, so unlike recognition there is no
/// async here. Concrete layers: [`FuseLayer`] (fuse each cluster of the same
/// finding), [`ReconcileLayer`] (keep, contest, or resolve each overlapping
/// pair), [`ThresholdLayer`] (drop by confidence), [`CalibrateLayer`] (rescale
/// confidence).
///
/// [`FuseLayer`]: overlap::FuseLayer
/// [`ReconcileLayer`]: overlap::ReconcileLayer
/// [`ThresholdLayer`]: filter::ThresholdLayer
/// [`CalibrateLayer`]: calibrate::CalibrateLayer
pub trait Layer<M: Modality>: Send + Sync {
    /// Apply this stage to `entities`.
    fn apply(&self, entities: Vec<Entity<M>>) -> LayerOutput<M>;
}
