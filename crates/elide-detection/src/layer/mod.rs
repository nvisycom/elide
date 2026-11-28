//! The [`Layer`] pipeline: post-detection stages that reshape and prune the
//! working entity set.
//!
//! Recognizers emit entities independently; these [`Layer`]s reshape and prune
//! them. They are composed onto an [`Analyzer`], which runs them in order after
//! detection. The shipped stages, in their usual order:
//!
//! 1. [`overlap`], decide what happens to overlapping entities: cluster them
//!    ([`GroupPredicate`]), fuse each cluster of the same finding
//!    ([`FuseLayer`]), and arbitrate cross-label pairs ([`Reconciler`]):
//!    keep, contest, or resolve.
//! 2. [`filter`], drop entities below a confidence threshold.
//!
//! A recognizer whose scores run differently from the others' is calibrated
//! before any layer, by wrapping it in a [`Calibrated`].
//!
//! Each stage implements [`Layer`], returning a [`LayerOutput`]. Stages are
//! pure and synchronous, reached through their submodules (e.g.
//! [`overlap::ReconcileLayer`]).
//!
//! [`Analyzer`]: crate::Analyzer
//! [`GroupPredicate`]: overlap::group::GroupPredicate
//! [`FuseLayer`]: overlap::FuseLayer
//! [`Reconciler`]: overlap::Reconciler
//! [`Calibrated`]: crate::decorator::calibrated::Calibrated

pub mod filter;
pub mod overlap;

use elide_core::Component;
use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::recognition::Context;
use uuid::Uuid;

/// What one [`Layer`] decided for each entity: kept, rejected, or superseded
/// by another.
///
/// A layer records each decision with [`keep`], [`reject`], or [`supersede`];
/// the analyzer threads the kept entities into the next layer and records a
/// [`Dropped`] event naming the layer on each of the rest.
///
/// [`Dropped`]: elide_core::entity::audit::Dropped
/// [`keep`]: Self::keep
/// [`reject`]: Self::reject
/// [`supersede`]: Self::supersede
#[derive(Debug)]
pub struct LayerOutput<M: Modality> {
    /// Entities that survived the layer.
    pub(crate) kept: Vec<Entity<M>>,
    /// Entities the layer removed, each with the entity that took its place,
    /// if any.
    pub(crate) dropped: Vec<(Entity<M>, Option<Uuid>)>,
}

impl<M: Modality> LayerOutput<M> {
    /// No decision yet.
    pub fn new() -> Self {
        Self {
            kept: Vec::new(),
            dropped: Vec::new(),
        }
    }

    /// Keep `entity`, passing it on to the next layer.
    pub fn keep(&mut self, entity: Entity<M>) {
        self.kept.push(entity);
    }

    /// Drop `entity` outright.
    pub fn reject(&mut self, entity: Entity<M>) {
        self.dropped.push((entity, None));
    }

    /// Drop `entity` in favor of the entity `winner`.
    pub fn supersede(&mut self, entity: Entity<M>, winner: Uuid) {
        self.dropped.push((entity, Some(winner)));
    }

    /// The entities kept so far.
    pub fn kept(&self) -> &[Entity<M>] {
        &self.kept
    }

    /// The entities dropped so far, each with the entity that took its place,
    /// if any.
    pub fn dropped(&self) -> &[(Entity<M>, Option<Uuid>)] {
        &self.dropped
    }
}

impl<M: Modality> Default for LayerOutput<M> {
    fn default() -> Self {
        Self::new()
    }
}

/// One stage of the layer pipeline: takes entities, keeps or drops them.
///
/// A layer takes the working set of entities and decides each one's fate in a
/// [`LayerOutput`]. Stages are **pure and synchronous**, they operate over a
/// `Vec<Entity<M>>` in memory with no I/O, so unlike recognition there is no
/// async here. They see the request's [`Context`], as recognizers do.
/// Concrete layers: [`FuseLayer`] (fuse each cluster of the same finding),
/// [`ReconcileLayer`] (keep, contest, or resolve each overlapping pair),
/// [`ThresholdLayer`] (drop by confidence), and [`CatalogFilter`] (drop what
/// the request did not ask for), which the analyzer runs after every other.
///
/// A layer is a [`Component`]: its [`id`](Component::id) names it on
/// everything it drops.
///
/// [`FuseLayer`]: overlap::FuseLayer
/// [`ReconcileLayer`]: overlap::ReconcileLayer
/// [`ThresholdLayer`]: filter::ThresholdLayer
/// [`CatalogFilter`]: filter::CatalogFilter
pub trait Layer<M: Modality>: Component + Send + Sync {
    /// Decide the fate of each of `entities` in the request `ctx`.
    fn apply(&self, entities: Vec<Entity<M>>, ctx: &Context<'_>) -> LayerOutput<M>;
}

#[cfg(test)]
pub(crate) use self::testing::Run;

/// Running a layer outside an analyzer, for the layers' unit tests.
#[cfg(test)]
mod testing {
    use elide_core::entity::Entity;
    use elide_core::modality::Modality;
    use elide_core::recognition::{Context, Scope};

    use super::{Layer, LayerOutput};

    /// A layer run outside an analyzer.
    pub(crate) trait Run<M: Modality> {
        /// Apply the layer to `entities` under an empty request.
        fn run(&self, entities: Vec<Entity<M>>) -> LayerOutput<M>;
    }

    impl<M: Modality, L: Layer<M>> Run<M> for L {
        fn run(&self, entities: Vec<Entity<M>>) -> LayerOutput<M> {
            self.apply(entities, &Context::new(&Scope::new()))
        }
    }
}
