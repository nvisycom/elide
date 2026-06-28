//! The [`Layer`] pipeline: post-detection stages that reshape and prune the
//! working entity set.
//!
//! Recognizers emit entities independently; these [`Layer`]s reshape and prune
//! them. They are composed onto an [`Analyzer`], which runs them in order after
//! detection. The shipped stages, in their usual order:
//!
//! 1. [`calibrate`] — scale each entity's confidence by a per-recognizer
//!    multiplier, so detectors with different score distributions are
//!    comparable before reconciliation.
//! 2. [`reconcile`] — decide what happens to overlapping entities: cluster
//!    them ([`GroupPredicate`]) and dispose of each pair ([`Reconciler`]) —
//!    merge same-label findings, keep/contest/resolve cross-label overlaps.
//! 3. [`filter`] — drop entities outside an allow-list of labels or below a
//!    confidence threshold.
//!
//! Each stage implements [`Layer`], returning a [`LayerOutput`]. Stages are
//! pure and synchronous, reached through their submodules (e.g.
//! [`reconcile::ReconcileLayer`]).
//!
//! [`Analyzer`]: crate::Analyzer
//! [`GroupPredicate`]: reconcile::group::GroupPredicate
//! [`Reconciler`]: reconcile::Reconciler

pub mod calibrate;
pub mod filter;
pub mod reconcile;

use elide_core::entity::Entity;
use elide_core::modality::Modality;

/// The result of running one [`Layer`].
///
/// Names the two outcomes explicitly so call sites don't juggle a positional
/// tuple, and so future per-stage metadata (counts, drop reasons) can be added
/// without changing every layer's signature. The pipeline threads [`kept`] into
/// the next layer and accumulates [`dropped`] for logging.
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
    /// All entities kept, none dropped — for layers that only reshape rather
    /// than remove.
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

/// One stage of the layer pipeline.
///
/// A layer takes the working set of entities and returns the reshaped set plus
/// anything it removed. Stages are **pure and synchronous** — they operate over
/// a `Vec<Entity<M>>` in memory with no I/O, so unlike recognition there is no
/// async here. Concrete layers: [`ReconcileLayer`] (cluster and dispose of
/// overlapping entities — merge, keep, contest, or resolve), [`FilterLayer`]
/// (drop by label / confidence), [`CalibrateLayer`] (rescale confidence).
///
/// [`ReconcileLayer`]: reconcile::ReconcileLayer
/// [`FilterLayer`]: filter::FilterLayer
/// [`CalibrateLayer`]: calibrate::CalibrateLayer
pub trait Layer<M: Modality>: Send + Sync {
    /// Apply this stage to `entities`.
    fn apply(&self, entities: Vec<Entity<M>>) -> LayerOutput<M>;
}
