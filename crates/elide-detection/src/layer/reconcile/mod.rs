//! Reconciliation: deciding what happens to overlapping entities.
//!
//! One layer, two axes. A [`GroupPredicate`](group::GroupPredicate) (`G`)
//! decides *which* entities cluster, and a
//! [`Reconciler`](reconciler::Reconciler) (`R`) decides *what to do* with a
//! grouped pair. Fusion and cross-label resolution are two configurations of
//! the same [`ReconcileLayer`]:
//!
//! - fusion = `ReconcileLayer<`[`LabelOverlap`](group::LabelOverlap)`,
//!   `[`Merging`](reconciler::Merging)`<…>>`
//! - resolution = `ReconcileLayer<`[`DiffLabelOverlap`](group::DiffLabelOverlap)`,
//!   `[`Structural`](reconciler::Structural)`<…>>`
//!
//! The pieces live in topic modules: [`group`] (the `G` axis), [`reconciler`]
//! (the `R` axis and the shipped reconcilers), [`scoring`] (how [`Merging`]
//! combines confidences), and [`tiebreaker`] (how conflicts pick a winner).
//!
//! [`Merging`]: reconciler::Merging

pub mod group;
pub mod reconciler;
pub mod scoring;
pub mod tiebreaker;

mod fold;

use elide_core::entity::Entity;
use elide_core::modality::Modality;

use self::fold::fold;
use self::group::{GroupPredicate, LabelOverlap};
use self::reconciler::{Merging, Reconciler};
use self::scoring::Max;
use super::{Layer, LayerOutput};

/// The reconciliation stage: cluster entities by `group`, then dispose of
/// each cluster's pairs with `reconciler`.
///
/// Generic over the grouping `G` and the reconciler `R`, each chosen at
/// construction. Same-label fusion and cross-label arbitration are both
/// expressed here; see the [module docs](self).
#[derive(Debug, Clone)]
pub struct ReconcileLayer<G, R> {
    group: G,
    reconciler: R,
}

impl<G, R> ReconcileLayer<G, R> {
    /// A reconcile layer clustering by `group` and disposing with
    /// `reconciler`.
    pub fn new(group: G, reconciler: R) -> Self {
        Self { group, reconciler }
    }
}

/// The standard same-label fusion pass: [`LabelOverlap`] + [`Merging`] scored
/// by [`Max`](scoring::Max).
impl Default for ReconcileLayer<LabelOverlap, Merging<Max>> {
    fn default() -> Self {
        Self::new(LabelOverlap, Merging::default())
    }
}

impl<M, G, R> Layer<M> for ReconcileLayer<G, R>
where
    M: Modality,
    G: GroupPredicate<M>,
    R: Reconciler<M>,
{
    fn apply(&self, entities: Vec<Entity<M>>) -> LayerOutput<M> {
        // Cluster by bucket (O(n)) then single-linkage by `is_grouped` within
        // each bucket. The bucket bounds the search: entities with different
        // buckets can never group (the trait law), so a clustered entity only
        // scans groups sharing its bucket.
        let mut buckets: Vec<(G::Bucket, Vec<Vec<usize>>)> = Vec::new();
        for (index, entity) in entities.iter().enumerate() {
            let bucket = self.group.bucket(entity);
            let groups = match buckets.iter_mut().find(|(b, _)| *b == bucket) {
                Some((_, groups)) => groups,
                None => {
                    buckets.push((bucket, Vec::new()));
                    &mut buckets.last_mut().expect("just pushed").1
                }
            };
            // Join every group this entity links to (single-linkage), merging
            // bridged groups; else start a new one.
            let hit: Vec<usize> = (0..groups.len())
                .filter(|&g| {
                    groups[g]
                        .iter()
                        .any(|&other| self.group.is_grouped(&entities[other], entity))
                })
                .collect();
            match hit.first().copied() {
                None => groups.push(vec![index]),
                Some(first) => {
                    groups[first].push(index);
                    for &g in hit.iter().skip(1).rev() {
                        let merged = groups.remove(g);
                        groups[first].extend(merged);
                    }
                }
            }
        }

        let clusters: Vec<Vec<usize>> =
            buckets.into_iter().flat_map(|(_, groups)| groups).collect();
        fold(&self.reconciler, entities, clusters)
    }
}
#[cfg(test)]
mod tests {
    use elide_core::entity::provenance::{Event, EventKind, PatternEvent, Provenance};
    use elide_core::entity::{Entity, LabelRef};
    use elide_core::modality::text::{Text, TextLocation};
    use elide_core::primitive::Confidence;

    use super::group::{DiffLabelOverlap, LabelOverlap};
    use super::reconciler::{Merging, Permissive, Structural};
    use super::scoring::NoisyOr;
    use super::*;

    fn entity(label: &str, start: usize, end: usize, conf: f32) -> Entity<Text> {
        let loc = TextLocation::new(start, end);
        let confidence = Confidence::new(conf).unwrap();
        let event = Event::pattern("t", confidence, loc.clone(), PatternEvent::default());
        Entity::new(
            LabelRef::new(label),
            loc,
            confidence,
            Provenance::new(event),
        )
    }

    fn has_kind(entity: &Entity<Text>, f: impl Fn(&EventKind<Text>) -> bool) -> bool {
        entity.provenance.events.iter().any(|e| f(&e.kind))
    }

    // --- fusion (LabelOverlap + Merging) ---

    /// Same-label overlapping findings merge into one entity spanning their
    /// union, with the pooled confidence.
    #[test]
    fn merges_same_label_into_the_union() {
        let entities = vec![entity("EMAIL", 0, 10, 0.9), entity("EMAIL", 6, 25, 0.6)];
        let out = ReconcileLayer::default().apply(entities);
        assert_eq!(out.kept.len(), 1);
        let s = &out.kept[0];
        assert_eq!((s.location.start, s.location.end), (0, 25));
        assert_eq!(s.label, LabelRef::new("EMAIL"));
        assert_eq!(s.confidence, Confidence::new(0.9).unwrap()); // max
    }

    /// `NoisyOr` accumulates: two agreeing 0.6 witnesses → 0.84.
    #[test]
    fn noisy_confidence_accumulates() {
        let entities = vec![entity("EMAIL", 0, 10, 0.6), entity("EMAIL", 0, 10, 0.6)];
        let layer = ReconcileLayer::new(LabelOverlap, Merging::new(NoisyOr));
        let out = layer.apply(entities);
        assert_eq!(out.kept.len(), 1);
        assert!((out.kept[0].confidence.get() - 0.84).abs() < 1e-5);
    }

    /// Different labels never group under `LabelOverlap`, so fusion leaves a
    /// cross-label overlap alone.
    #[test]
    fn fusion_ignores_different_labels() {
        let entities = vec![entity("EMAIL", 0, 10, 0.9), entity("PHONE", 0, 10, 0.8)];
        let out = ReconcileLayer::default().apply(entities);
        assert_eq!(out.kept.len(), 2);
    }

    // --- resolution (DiffLabelOverlap + Structural) ---

    fn structural() -> ReconcileLayer<DiffLabelOverlap, Structural> {
        ReconcileLayer::new(DiffLabelOverlap, Structural::default())
    }

    /// A nesting of different labels is a hierarchy — keep both.
    #[test]
    fn structural_keeps_nested() {
        let entities = vec![
            entity("ADDRESS", 0, 30, 0.8),
            entity("POSTAL_CODE", 20, 28, 0.9),
        ];
        let out = structural().apply(entities);
        assert_eq!(out.kept.len(), 2);
    }

    /// A weak contained match subsumed by a strong container is dropped.
    #[test]
    fn structural_drops_subsumed_weak() {
        let entities = vec![
            entity("iban", 0, 27, 0.85),
            entity("drivers_license", 0, 4, 0.4),
        ];
        let out = structural().apply(entities);
        assert_eq!(out.kept.len(), 1);
        assert_eq!(out.kept[0].label, LabelRef::new("iban"));
        assert_eq!(out.dropped.len(), 1);
    }

    /// A near-coincident cross-label overlap is a true conflict: the winner
    /// survives and records the loser; the loser is dropped.
    #[test]
    fn structural_resolves_true_conflict() {
        let entities = vec![
            entity("PERSON_NAME", 0, 12, 0.9),
            entity("ORGANIZATION", 2, 14, 0.7),
        ];
        let out = structural().apply(entities);
        assert_eq!(out.kept.len(), 1);
        assert_eq!(out.dropped.len(), 1);
        assert_eq!(out.kept[0].label, LabelRef::new("PERSON_NAME"));
        assert!(has_kind(&out.kept[0], |k| matches!(
            k,
            EventKind::Conflict { competing_label, .. } if *competing_label == LabelRef::new("ORGANIZATION")
        )));
    }

    /// With `OnConflict::Contest`, a true conflict keeps BOTH, each flagged
    /// contested against the other — nothing dropped.
    #[test]
    fn structural_contests_when_reviewing() {
        let entities = vec![
            entity("PERSON_NAME", 0, 12, 0.9),
            entity("ORGANIZATION", 2, 14, 0.7),
        ];
        let layer = ReconcileLayer::new(DiffLabelOverlap, Structural::reviewing());
        let out = layer.apply(entities);
        assert_eq!(out.kept.len(), 2, "contest keeps both");
        assert!(out.dropped.is_empty());
        for e in &out.kept {
            assert!(has_kind(e, |k| matches!(k, EventKind::Contested { .. })));
        }
    }

    /// `Permissive` keeps every cross-label overlap untouched (Presidio-style).
    #[test]
    fn permissive_keeps_everything() {
        let entities = vec![
            entity("PERSON_NAME", 0, 12, 0.9),
            entity("ORGANIZATION", 2, 14, 0.7),
        ];
        let out = ReconcileLayer::new(DiffLabelOverlap, Permissive::new()).apply(entities);
        assert_eq!(out.kept.len(), 2);
        assert!(out.dropped.is_empty());
    }
}
