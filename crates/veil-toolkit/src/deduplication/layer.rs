//! The [`Layer`] trait and [`LayerOutput`] — one stage of deduplication.

use veil_core::entity::Entity;
use veil_core::modality::Modality;

/// The result of running one deduplication [`Layer`].
///
/// Names the two outcomes explicitly so call sites don't juggle a
/// positional tuple, and so future per-stage metadata (counts, drop
/// reasons) can be added without changing every layer's signature. The
/// pipeline threads [`kept`] into the next layer and accumulates
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
    /// All entities kept, none dropped — for layers that only reshape
    /// (fuse) rather than remove.
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

/// One stage of the deduplication pipeline.
///
/// A layer takes the working set of entities and returns the reshaped
/// set plus anything it removed. Stages are **pure and synchronous** —
/// they operate over a `Vec<Entity<M>>` in memory with no I/O, so unlike
/// recognition there is no async here. Concrete layers: [`FuseLayer`]
/// (combine co-located findings), [`ResolveLayer`] (break cross-label
/// overlaps), [`FilterLayer`] (drop by label / confidence),
/// [`CalibrateLayer`] (rescale confidence).
///
/// [`FuseLayer`]: crate::deduplication::fuse::FuseLayer
/// [`ResolveLayer`]: crate::deduplication::resolve::ResolveLayer
/// [`FilterLayer`]: crate::deduplication::filter::FilterLayer
/// [`CalibrateLayer`]: crate::deduplication::calibrate::CalibrateLayer
pub trait Layer<M: Modality>: Send + Sync {
    /// Apply this stage to `entities`.
    fn apply(&self, entities: Vec<Entity<M>>) -> LayerOutput<M>;
}
