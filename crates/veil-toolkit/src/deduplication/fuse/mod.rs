//! Fusion: combining co-located findings of the same label into one
//! entity. Holds the [`FusionStrategy`] (how scores combine), the
//! [`GroupPredicate`] (which entities are "the same"), and the
//! [`FuseLayer`] that drives them.

mod group;
mod strategy;

use veil_core::entity::Entity;
use veil_core::modality::{Modality, ModalityLocation};
use veil_core::provenance::Event;

pub use self::group::{GroupPredicate, SameLabelOverlap};
pub use self::strategy::{FusionStrategy, MaxConfidence, Mean, NoisyOr};
use super::{Layer, LayerOutput};

/// The fusion stage: clusters entities by a [`GroupPredicate`] and
/// combines each cluster into one entity with a [`FusionStrategy`].
///
/// Generic over both the strategy `S` and the predicate `G` so each is a
/// type chosen at construction.
#[derive(Debug, Clone)]
pub struct FuseLayer<S, G> {
    strategy: S,
    group: G,
}

impl<S> FuseLayer<S, SameLabelOverlap> {
    /// A fuse layer using `strategy` to combine and the default
    /// same-label/overlap grouping. Swap the grouping with
    /// [`with_group`](FuseLayer::with_group).
    pub fn new(strategy: S) -> Self {
        Self {
            strategy,
            group: SameLabelOverlap,
        }
    }
}

impl<S, G> FuseLayer<S, G> {
    /// Replace the grouping predicate, consuming and returning `self`.
    #[must_use]
    pub fn with_group<G2>(self, group: G2) -> FuseLayer<S, G2> {
        FuseLayer {
            strategy: self.strategy,
            group,
        }
    }
}

impl<M, S, G> Layer<M> for FuseLayer<S, G>
where
    M: Modality,
    S: FusionStrategy<M>,
    G: GroupPredicate<M>,
{
    fn apply(&self, entities: Vec<Entity<M>>) -> LayerOutput<M> {
        // Single-linkage clustering by the predicate: each entity joins
        // the first existing group it matches, else starts a new one.
        let mut groups: Vec<Vec<Entity<M>>> = Vec::new();
        for entity in entities {
            match groups
                .iter_mut()
                .find(|g| g.iter().any(|m| self.group.same(m, &entity)))
            {
                Some(group) => group.push(entity),
                None => groups.push(vec![entity]),
            }
        }

        let kept = groups
            .into_iter()
            .map(|group| fuse_group(&self.strategy, group))
            .collect();
        // Fusion reshapes rather than drops: the absorbed entities live
        // on inside the survivor's provenance.
        LayerOutput::kept(kept)
    }
}

/// Combine a cluster into one entity: pick the highest-confidence base,
/// adopt the largest location, concatenate every contributing entity's
/// provenance events, set the fused confidence, and append a
/// deduplication [`Event`].
fn fuse_group<M, S>(strategy: &S, mut group: Vec<Entity<M>>) -> Entity<M>
where
    M: Modality,
    S: FusionStrategy<M>,
{
    if group.len() == 1 {
        return group.pop().expect("len == 1");
    }

    let after = strategy.confidence(&group);

    // Highest-confidence entity becomes the base.
    group.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut base = group.remove(0);
    let before = base.confidence;

    // Adopt the largest location among the cluster.
    for other in &group {
        if other.location.span_cmp(&base.location) == std::cmp::Ordering::Greater {
            base.location = other.location.clone();
        }
    }

    // Concatenate every contributing entity's provenance events.
    for other in group {
        base.provenance.events.extend(other.provenance.events);
    }

    base.confidence = after;
    base.provenance
        .record(Event::deduplication(strategy.name(), before, after));
    base
}
