//! Folding pairwise [`Disposition`]s over a cluster into kept/dropped
//! entities.

use std::mem;

use elide_core::entity::Entity;
use elide_core::entity::provenance::Event;
use elide_core::modality::{Modality, ModalityLocation};
use elide_core::primitive::Confidence;

use super::Cluster;
use super::reconciler::{Disposition, Reconciler, Winner};
use crate::layer::LayerOutput;

/// Fold each cluster's pairwise dispositions into kept/dropped entities.
///
/// Precedence within a cluster: `Resolve` drops losers first (so a span about
/// to be discarded isn't merged), then `Contest` flags survivors, then `Merge`
/// unions the survivors still linked by a merge.
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
    let mut merges: Vec<(usize, usize, Confidence)> = Vec::new();

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
                    Disposition::Merge { confidence } => merges.push((i, j, confidence)),
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
    apply_merges(&mut entities, &mut dropped, &merges, name);

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
        let event = Event::conflict(
            name.to_owned(),
            entities[winner].confidence,
            entities[loser].label.clone(),
            entities[loser].confidence,
        );
        entities[winner].provenance.record(event);
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
        let on_a = Event::contested(
            name.to_owned(),
            entities[a].confidence,
            entities[b].label.clone(),
            entities[b].confidence,
        );
        let on_b = Event::contested(
            name.to_owned(),
            entities[b].confidence,
            entities[a].label.clone(),
            entities[a].confidence,
        );
        entities[a].provenance.record(on_a);
        entities[b].provenance.record(on_b);
    }
}

/// Apply merges: union each pair into a single survivor (the higher-confidence
/// side is the base), folding the other in and dropping it. Pooled confidence
/// comes from the disposition.
fn apply_merges<M: Modality>(
    entities: &mut [Entity<M>],
    dropped: &mut [bool],
    merges: &[(usize, usize, Confidence)],
    name: &str,
) {
    for &(i, j, confidence) in merges {
        if dropped[i] || dropped[j] {
            continue;
        }
        // The higher-confidence side is the base (donates label + scalar
        // fields); the other folds in and is dropped.
        let (base, other) = if entities[i].confidence >= entities[j].confidence {
            (i, j)
        } else {
            (j, i)
        };
        let before = entities[base].confidence;
        if let Some(union) = entities[base].location.union(&entities[other].location) {
            entities[base].location = union;
        }
        let other_events = mem::take(&mut entities[other].provenance.events);
        entities[base].provenance.events.extend(other_events);
        entities[base].confidence = confidence;
        entities[base]
            .provenance
            .record(Event::deduplication(name.to_owned(), before, confidence));
        dropped[other] = true;
    }
}
