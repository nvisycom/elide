//! Folding pairwise [`Disposition`]s over a cluster into kept/dropped
//! entities.

use elide_core::entity::Entity;
use elide_core::entity::audit::{AuditEvent, Contested, Resolved};
use elide_core::modality::Modality;
use elide_core::primitive::ComponentId;

use super::Cluster;
use super::reconciler::{Disposition, Reconciler, Winner};
use crate::layer::LayerOutput;

/// Fold each cluster's pairwise dispositions into kept/dropped entities.
///
/// Precedence within a cluster: `Resolve` drops losers first, then `Contest`
/// flags the pairs that both survive. Each loser is dropped in favor of the
/// winner that beat it.
pub(super) fn fold<M, R>(
    reconciler: &R,
    mut entities: Vec<Entity<M>>,
    clusters: Vec<Cluster>,
) -> LayerOutput<M>
where
    M: Modality,
    R: Reconciler<M>,
{
    let mut verdicts = Verdicts::new(entities.len());
    for cluster in &clusters {
        for (pi, &i) in cluster.iter().enumerate() {
            for &j in &cluster[pi + 1..] {
                if verdicts.is_dropped(i) {
                    break; // i is gone; stop pairing it
                }
                if verdicts.is_dropped(j) {
                    continue;
                }
                match reconciler.decide(&entities[i], &entities[j]) {
                    Disposition::KeepBoth => {}
                    Disposition::Contest => verdicts.contests.push((i, j)),
                    Disposition::Resolve { winner } => match winner {
                        Winner::First => verdicts.resolve(i, j),
                        Winner::Second => verdicts.resolve(j, i),
                    },
                }
            }
        }
    }

    let by = reconciler.id();
    verdicts.record_conflicts(&mut entities, &by);
    verdicts.record_contests(&mut entities, &by);
    verdicts.split(entities)
}

/// What folding a set of entities decided: which entity beat each loser, and
/// which pairs were left contested. The provenance is written after the
/// read-only decision pass.
struct Verdicts {
    /// The index of the entity that beat each one, `None` for a survivor.
    beaten_by: Vec<Option<usize>>,
    /// Pairs both flagged.
    contests: Vec<(usize, usize)>,
}

impl Verdicts {
    /// No verdict yet over `len` entities.
    fn new(len: usize) -> Self {
        Self {
            beaten_by: vec![None; len],
            contests: Vec::new(),
        }
    }

    /// Whether the entity at `index` lost.
    fn is_dropped(&self, index: usize) -> bool {
        self.beaten_by[index].is_some()
    }

    /// `winner` beat `loser`, which is dropped.
    fn resolve(&mut self, winner: usize, loser: usize) {
        self.beaten_by[loser] = Some(winner);
    }

    /// Record on each winner the loser it beat, as `by` resolved it. A winner
    /// that later lost elsewhere still records its win, so its own drop reads
    /// whole.
    fn record_conflicts<M: Modality>(&self, entities: &mut [Entity<M>], by: &ComponentId) {
        for (loser, winner) in self.beaten_by.iter().enumerate() {
            let Some(winner) = *winner else { continue };
            let conflict = Resolved::new(
                entities[loser].id(),
                entities[loser].label().clone(),
                entities[loser].confidence(),
                by.clone(),
            );
            let event = AuditEvent::new(entities[winner].confidence(), conflict);
            entities[winner].record(event);
        }
    }

    /// Flag each contested pair on both entities (only if both survive), as
    /// `by` left it.
    fn record_contests<M: Modality>(&self, entities: &mut [Entity<M>], by: &ComponentId) {
        for &(a, b) in &self.contests {
            if self.is_dropped(a) || self.is_dropped(b) {
                continue;
            }
            let on_a = AuditEvent::new(
                entities[a].confidence(),
                Contested::new(
                    entities[b].id(),
                    entities[b].label().clone(),
                    entities[b].confidence(),
                    by.clone(),
                ),
            );
            let on_b = AuditEvent::new(
                entities[b].confidence(),
                Contested::new(
                    entities[a].id(),
                    entities[a].label().clone(),
                    entities[a].confidence(),
                    by.clone(),
                ),
            );
            entities[a].record(on_a);
            entities[b].record(on_b);
        }
    }

    /// Keep each survivor, and drop each loser in favor of its winner.
    fn split<M: Modality>(self, entities: Vec<Entity<M>>) -> LayerOutput<M> {
        let ids: Vec<_> = entities.iter().map(Entity::id).collect();
        let mut output = LayerOutput::new();
        for (entity, winner) in entities.into_iter().zip(self.beaten_by) {
            match winner {
                Some(winner) => output.supersede(entity, ids[winner]),
                None => output.keep(entity),
            }
        }
        output
    }
}
