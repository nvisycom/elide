//! Conflict resolution: breaking overlaps between entities of
//! *different* labels (same-label overlaps are handled by fusion).

mod strategy;

use elide_core::entity::Entity;
use elide_core::modality::{Modality, ModalityLocation};

pub use self::strategy::{ConflictResolution, HighestConfidence, LongestSpan};
use super::{Layer, LayerOutput};

/// The conflict-resolution stage: where two entities of *different*
/// labels overlap, drop the loser per the [`ConflictResolution`]
/// strategy.
///
/// Any overlap between different labels is treated as a conflict (this
/// does not model legitimate nesting — that refinement is left to the
/// strategy or a future stage). Same-label overlaps are never touched;
/// fusion owns those.
#[derive(Debug, Clone)]
pub struct ResolveLayer<R> {
    strategy: R,
}

impl<R> ResolveLayer<R> {
    /// A resolve layer using `strategy` to pick winners.
    pub fn new(strategy: R) -> Self {
        Self { strategy }
    }
}

impl<M, R> Layer<M> for ResolveLayer<R>
where
    M: Modality,
    R: ConflictResolution<M>,
{
    fn apply(&self, entities: Vec<Entity<M>>) -> LayerOutput<M> {
        let len = entities.len();
        let mut is_loser = vec![false; len];

        for i in 0..len {
            if is_loser[i] {
                continue;
            }
            for j in (i + 1)..len {
                if is_loser[j] {
                    continue;
                }
                // Same label → fusion's concern, not resolve's.
                if entities[i].label == entities[j].label {
                    continue;
                }
                // No overlap → no conflict.
                if !entities[i].location.overlaps(&entities[j].location) {
                    continue;
                }
                if self.strategy.keeps_first(&entities[i], &entities[j]) {
                    is_loser[j] = true;
                } else {
                    is_loser[i] = true;
                    break; // i is gone; stop comparing it against later j
                }
            }
        }

        let mut kept = Vec::new();
        let mut dropped = Vec::new();
        for (entity, loser) in entities.into_iter().zip(is_loser) {
            if loser {
                dropped.push(entity);
            } else {
                kept.push(entity);
            }
        }
        LayerOutput::split(kept, dropped)
    }
}
