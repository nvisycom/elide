//! Folding pairwise [`Disposition`]s over a cluster into kept/dropped
//! entities.

use elide_core::entity::Entity;
use elide_core::entity::audit::{AuditEvent, Conflict, Contested};
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
    let len = entities.len();
    let mut dropped = vec![false; len];
    // Deferred provenance writes, applied after the read-only decision pass.
    let mut conflicts: Vec<(usize, usize)> = Vec::new(); // (winner, loser)
    let mut contests: Vec<(usize, usize)> = Vec::new(); // both flagged

    for cluster in &clusters {
        for (pi, &i) in cluster.iter().enumerate() {
            for &j in &cluster[pi + 1..] {
                if dropped[i] {
                    break; // i is gone; stop pairing it
                }
                if dropped[j] {
                    continue;
                }
                match reconciler.decide(&entities[i], &entities[j]) {
                    Disposition::KeepBoth => {}
                    Disposition::Contest => contests.push((i, j)),
                    Disposition::Resolve { winner } => match winner {
                        Winner::First => {
                            dropped[j] = true;
                            conflicts.push((i, j));
                        }
                        Winner::Second => {
                            dropped[i] = true;
                            conflicts.push((j, i));
                        }
                    },
                }
            }
        }
    }

    let name = reconciler.name();
    record_conflicts(&mut entities, &dropped, &conflicts, name);
    record_contests(&mut entities, &dropped, &contests, name);

    let mut kept = Vec::new();
    let mut out_dropped = Vec::new();
    for (entity, is_dropped) in entities.into_iter().zip(dropped) {
        if is_dropped {
            out_dropped.push(entity);
        } else {
            kept.push(entity);
        }
    }
    LayerOutput::split(kept, out_dropped)
}

/// Record each surviving winner's conflict claim (the dropped loser).
fn record_conflicts<M: Modality>(
    entities: &mut [Entity<M>],
    dropped: &[bool],
    conflicts: &[(usize, usize)],
    name: &str,
) {
    for &(winner, loser) in conflicts {
        if dropped[winner] {
            continue; // winner later lost elsewhere; its claim travels nowhere
        }
        let conflict = Conflict::new(
            entities[loser].label.clone(),
            entities[loser].confidence,
            name.to_owned(),
        );
        let event = AuditEvent::conflict(conflict, entities[winner].confidence);
        entities[winner].record(event);
    }
}

/// Flag each contested pair on both entities (only if both survive).
fn record_contests<M: Modality>(
    entities: &mut [Entity<M>],
    dropped: &[bool],
    contests: &[(usize, usize)],
    name: &str,
) {
    for &(a, b) in contests {
        if dropped[a] || dropped[b] {
            continue;
        }
        let on_a = AuditEvent::contested(
            Contested::new(
                entities[b].label.clone(),
                entities[b].confidence,
                name.to_owned(),
            ),
            entities[a].confidence,
        );
        let on_b = AuditEvent::contested(
            Contested::new(
                entities[a].label.clone(),
                entities[a].confidence,
                name.to_owned(),
            ),
            entities[b].confidence,
        );
        entities[a].record(on_a);
        entities[b].record(on_b);
    }
}
