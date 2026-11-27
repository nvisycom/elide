//! Folding pairwise [`Disposition`]s over a cluster into kept/dropped
//! entities.

use elide_core::entity::Entity;
use elide_core::entity::audit::{AuditEvent, Contested, Resolved};
use elide_core::modality::Modality;

use super::Cluster;
use super::reconciler::{Disposition, Reconciler, Winner};
use crate::layer::LayerOutput;

/// Fold each cluster's pairwise dispositions into kept/dropped entities.
///
/// Precedence within a cluster: `Resolve` drops losers first, then `Contest`
/// flags the pairs that both survive.
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
                if verdicts.dropped[i] {
                    break; // i is gone; stop pairing it
                }
                if verdicts.dropped[j] {
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

    let name = reconciler.name();
    verdicts.record_conflicts(&mut entities, name);
    verdicts.record_contests(&mut entities, name);
    verdicts.split(entities)
}

/// What folding a set of entities decided: which were dropped, which winners
/// beat which losers, and which pairs were left contested. The provenance is
/// written after the read-only decision pass.
struct Verdicts {
    dropped: Vec<bool>,
    /// `(winner, loser)` pairs.
    conflicts: Vec<(usize, usize)>,
    /// Pairs both flagged.
    contests: Vec<(usize, usize)>,
}

impl Verdicts {
    /// No verdict yet over `len` entities.
    fn new(len: usize) -> Self {
        Self {
            dropped: vec![false; len],
            conflicts: Vec::new(),
            contests: Vec::new(),
        }
    }

    /// `winner` beat `loser`, which is dropped.
    fn resolve(&mut self, winner: usize, loser: usize) {
        self.dropped[loser] = true;
        self.conflicts.push((winner, loser));
    }

    /// Record each surviving winner's conflict claim (the dropped loser), as
    /// the `name` reconciler resolved it.
    fn record_conflicts<M: Modality>(&self, entities: &mut [Entity<M>], name: &str) {
        for &(winner, loser) in &self.conflicts {
            if self.dropped[winner] {
                continue; // winner later lost elsewhere; its claim travels nowhere
            }
            let conflict = Resolved::new(
                entities[loser].label().clone(),
                entities[loser].confidence(),
                name.to_owned(),
            );
            let event = AuditEvent::resolved(conflict, entities[winner].confidence());
            entities[winner].record(event);
        }
    }

    /// Flag each contested pair on both entities (only if both survive), as
    /// the `name` reconciler left it.
    fn record_contests<M: Modality>(&self, entities: &mut [Entity<M>], name: &str) {
        for &(a, b) in &self.contests {
            if self.dropped[a] || self.dropped[b] {
                continue;
            }
            let on_a = AuditEvent::contested(
                Contested::new(
                    entities[b].label().clone(),
                    entities[b].confidence(),
                    name.to_owned(),
                ),
                entities[a].confidence(),
            );
            let on_b = AuditEvent::contested(
                Contested::new(
                    entities[a].label().clone(),
                    entities[a].confidence(),
                    name.to_owned(),
                ),
                entities[b].confidence(),
            );
            entities[a].record(on_a);
            entities[b].record(on_b);
        }
    }

    /// Split `entities` into the kept and the dropped.
    fn split<M: Modality>(self, entities: Vec<Entity<M>>) -> LayerOutput<M> {
        let mut kept = Vec::new();
        let mut out_dropped = Vec::new();
        for (entity, is_dropped) in entities.into_iter().zip(self.dropped) {
            if is_dropped {
                out_dropped.push(entity);
            } else {
                kept.push(entity);
            }
        }
        LayerOutput::split(kept, out_dropped)
    }
}
