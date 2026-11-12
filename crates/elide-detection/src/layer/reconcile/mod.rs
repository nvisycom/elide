//! Reconciliation: deciding what happens to overlapping entities.
//!
//! A [`GroupPredicate`] decides *which* entities cluster; two layers then act
//! on the clusters:
//!
//! - [`FuseLayer`] fuses each cluster into one entity, over the union of its
//!   spans, pooling confidence with a [`Scoring`](scoring::Scoring): the same
//!   finding reported more than once (by default, [`SameLabel`](group::SameLabel)
//!   overlaps).
//! - [`ReconcileLayer`] arbitrates each pair in a cluster with a
//!   [`Reconciler`]: keep both, contest, or resolve to a winner (by default,
//!   [`CrossLabel`] overlaps).
//!
//! The [`Reconciler`]s, [`Structural`], [`Exclusive`], [`Permissive`], sit here
//! at the root. The customization traits live in topic modules: [`group`] (which
//! entities cluster), [`scoring`] (how fusion pools confidences), and
//! [`tiebreaker`] (how a conflict picks a winner).

pub mod group;
pub mod scoring;
pub mod tiebreaker;

mod fold;
mod fuse;
mod reconciler;

use std::collections::HashMap;

use elide_core::entity::Entity;
use elide_core::modality::Modality;

use self::fold::fold;
pub use self::fuse::FuseLayer;
use self::group::{CrossLabel, GroupPredicate};
pub use self::reconciler::{
    Disposition, Exclusive, OnConflict, Permissive, Reconciler, Structural, Winner,
};
use super::{Layer, LayerOutput};

/// One cluster: the indices (into the working `entities`) of entities that
/// single-linkage-grouped together, e.g. three overlapping `EMAIL`s become
/// `[2, 5, 7]`. Clustering works on indices so entities aren't moved or cloned
/// while groups are built and merged.
type Cluster = Vec<usize>;

/// Cluster `entities` by `group`: bucket each (O(1) by `bucket`), then
/// single-linkage by `is_grouped` within its bucket. Every entity lands in
/// exactly one cluster, a lone entity in a cluster of its own.
///
/// The bucket bounds the search: entities with different buckets can never
/// group (the trait law), so an entity only scans clusters sharing its bucket.
/// Within a bucket, overlap does the fine grouping, so one bucket can hold
/// several disjoint clusters (two overlapping emails here, two unrelated ones
/// elsewhere). Bucket iteration order doesn't affect the result: clusters are
/// independent, and each layer rebuilds its output in entity order.
fn cluster<M, G>(group: &G, entities: &[Entity<M>]) -> Vec<Cluster>
where
    M: Modality,
    G: GroupPredicate<M>,
{
    let mut buckets: HashMap<G::Bucket, Vec<Cluster>> = HashMap::new();
    for (index, entity) in entities.iter().enumerate() {
        let groups = buckets.entry(group.bucket(entity)).or_default();
        // Join every group this entity links to (single-linkage), merging
        // bridged groups; else start a new one.
        let hit: Vec<usize> = (0..groups.len())
            .filter(|&g| {
                groups[g]
                    .iter()
                    .any(|&other| group.is_grouped(&entities[other], entity))
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
    buckets.into_values().flatten().collect()
}

/// The arbitration stage: settles each pair of overlapping, differing findings.
///
/// It clusters overlapping entities, then disposes of each pair with a
/// [`Reconciler`] (keep both, contest, or resolve to a winner).
///
/// Generic over the grouping `G` and the reconciler `R`, each chosen at
/// construction; see the [module docs].
///
/// [module docs]: self
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

impl<R> ReconcileLayer<CrossLabel, R> {
    /// A reconcile layer over *cross-label* overlaps, the conflict pass.
    /// Clusters overlapping findings of different labels and disposes of each
    /// pair with `reconciler` (typically a [`Structural`]).
    pub fn cross_label(reconciler: R) -> Self {
        Self::new(CrossLabel, reconciler)
    }
}

impl<M, G, R> Layer<M> for ReconcileLayer<G, R>
where
    M: Modality,
    G: GroupPredicate<M>,
    R: Reconciler<M>,
{
    fn apply(&self, entities: Vec<Entity<M>>) -> LayerOutput<M> {
        let clusters = cluster(&self.group, &entities);
        fold(&self.reconciler, entities, clusters)
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::audit::{AuditEvent, AuditKind, PatternRule};
    use elide_core::entity::{Entity, LabelRef};
    use elide_core::modality::text::{Text, TextLocation};
    use elide_core::primitive::Confidence;

    use super::group::CrossLabel;
    use super::*;

    fn entity(label: &str, start: usize, end: usize, conf: f32) -> Entity<Text> {
        let loc = TextLocation::inline(start..end);
        let confidence = Confidence::new(conf).unwrap();
        let event = AuditEvent::pattern("t", confidence, loc.clone(), PatternRule::default());
        Entity::new(LabelRef::new(label), loc).with_event(event)
    }

    fn has_kind(entity: &Entity<Text>, f: impl Fn(&AuditKind<Text>) -> bool) -> bool {
        entity.audit.events().iter().any(|e| f(&e.kind))
    }

    fn structural() -> ReconcileLayer<CrossLabel, Structural> {
        ReconcileLayer::cross_label(Structural::default())
    }

    /// A nesting of different labels is a hierarchy, keep both.
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

    /// A contained match exactly `margin` (0.25) weaker than its container is
    /// subsumed, not kept: the boundary is inclusive, so a junk sub-match (a
    /// digit run inside a payment card) does not slip through by a hair.
    #[test]
    fn structural_drops_contained_at_margin_boundary() {
        let entities = vec![
            entity("payment_card", 0, 17, 0.65),
            entity("postal_code", 5, 11, 0.40), // 0.40 + 0.25 == 0.65
        ];
        let out = structural().apply(entities);
        assert_eq!(out.kept.len(), 1);
        assert_eq!(out.kept[0].label, LabelRef::new("payment_card"));
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
            AuditKind::Conflict(c) if c.competing_label == LabelRef::new("ORGANIZATION")
        )));
    }

    /// With `OnConflict::Contest`, a true conflict keeps BOTH, each flagged
    /// contested against the other, nothing dropped.
    #[test]
    fn structural_contests_when_reviewing() {
        let entities = vec![
            entity("PERSON_NAME", 0, 12, 0.9),
            entity("ORGANIZATION", 2, 14, 0.7),
        ];
        let layer = ReconcileLayer::cross_label(Structural::standard().reviewing());
        let out = layer.apply(entities);
        assert_eq!(out.kept.len(), 2, "contest keeps both");
        assert!(out.dropped.is_empty());
        for e in &out.kept {
            assert!(has_kind(e, |k| matches!(k, AuditKind::Contested(_))));
        }
    }

    /// `Permissive` keeps every cross-label overlap untouched (Presidio-style).
    #[test]
    fn permissive_keeps_everything() {
        let entities = vec![
            entity("PERSON_NAME", 0, 12, 0.9),
            entity("ORGANIZATION", 2, 14, 0.7),
        ];
        let out = ReconcileLayer::cross_label(Permissive::new()).apply(entities);
        assert_eq!(out.kept.len(), 2);
        assert!(out.dropped.is_empty());
    }
}
