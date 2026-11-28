//! Overlapping findings: deciding what happens to entities that overlap.
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

use elide_core::Component;
use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::recognition::Context;

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
/// Generic over the grouping `G` (by default [`CrossLabel`]: different
/// labels, overlapping) and the reconciler `R`; see the [module docs]. Each
/// loser is dropped in its winner's favor. The layer is identified by its
/// [`Reconciler`].
///
/// [module docs]: self
#[derive(Debug, Clone, Copy, Default, Component)]
pub struct ReconcileLayer<G = CrossLabel, R = Structural> {
    group: G,
    #[component(delegate)]
    reconciler: R,
}

impl<R> ReconcileLayer<CrossLabel, R> {
    /// A reconcile layer over cross-label overlaps, disposing of each pair
    /// with `reconciler` (typically a [`Structural`]).
    pub fn new(reconciler: R) -> Self {
        Self {
            group: CrossLabel,
            reconciler,
        }
    }
}

impl ReconcileLayer<CrossLabel, Structural> {
    /// Arbitrate cross-label overlaps by how their spans relate (the default):
    /// keep nestings, resolve conflicts to the more confident finding.
    pub fn structural() -> Self {
        Self::new(Structural::standard())
    }
}

impl ReconcileLayer<CrossLabel, Exclusive> {
    /// Keep one finding per cross-label overlap, the more confident one.
    pub fn exclusive() -> Self {
        Self::new(Exclusive::highest_confidence())
    }
}

impl ReconcileLayer<CrossLabel, Permissive> {
    /// Keep every cross-label overlap untouched.
    pub fn permissive() -> Self {
        Self::new(Permissive::new())
    }
}

impl<G, R> ReconcileLayer<G, R> {
    /// Cluster by `group` instead: which findings are arbitrated as a pair.
    #[must_use]
    pub fn with_grouping<G2>(self, group: G2) -> ReconcileLayer<G2, R> {
        ReconcileLayer {
            group,
            reconciler: self.reconciler,
        }
    }
}

impl<M, G, R> Layer<M> for ReconcileLayer<G, R>
where
    M: Modality,
    G: GroupPredicate<M>,
    R: Reconciler<M>,
{
    fn apply(&self, entities: Vec<Entity<M>>, _ctx: &Context<'_>) -> LayerOutput<M> {
        let clusters = cluster(&self.group, &entities);
        fold(&self.reconciler, entities, clusters)
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use elide_core::entity::audit::{AuditKind, Pattern, PatternRule};
    use elide_core::entity::{Birth, Entity, LabelRef};
    use elide_core::modality::text::{Text, TextLocation};
    use elide_core::primitive::Confidence;

    use super::group::CrossLabel;
    use super::*;
    use crate::layer::Run;

    fn entity(label: &str, span: Range<usize>, conf: f32) -> Entity<Text> {
        let loc = TextLocation::inline(span);
        let confidence = Confidence::new(conf).unwrap();
        Entity::new(Birth::new(
            "t",
            confidence,
            Pattern::new(LabelRef::new(label), loc, PatternRule::default()),
        ))
    }

    fn has_kind(entity: &Entity<Text>, f: impl Fn(&AuditKind<Text>) -> bool) -> bool {
        entity.audit().events().iter().any(|e| f(&e.kind))
    }

    fn structural() -> ReconcileLayer<CrossLabel, Structural> {
        ReconcileLayer::structural()
    }

    /// A nesting of different labels is a hierarchy, keep both.
    #[test]
    fn structural_keeps_nested() {
        let entities = vec![
            entity("ADDRESS", 0..30, 0.8),
            entity("POSTAL_CODE", 20..28, 0.9),
        ];
        let out = structural().run(entities);
        assert_eq!(out.kept().len(), 2);
    }

    /// A weak contained match subsumed by a strong container is dropped.
    #[test]
    fn structural_drops_subsumed_weak() {
        let entities = vec![
            entity("iban", 0..27, 0.85),
            entity("drivers_license", 0..4, 0.4),
        ];
        let out = structural().run(entities);
        assert_eq!(out.kept().len(), 1);
        assert_eq!(*out.kept()[0].label(), LabelRef::new("iban"));
        assert_eq!(out.dropped().len(), 1);
    }

    /// A contained match exactly `margin` (0.25) weaker than its container is
    /// subsumed, not kept: the boundary is inclusive, so a junk sub-match (a
    /// digit run inside a payment card) does not slip through by a hair.
    #[test]
    fn structural_drops_contained_at_margin_boundary() {
        let entities = vec![
            entity("payment_card", 0..17, 0.65),
            entity("postal_code", 5..11, 0.40), // 0.40 + 0.25 == 0.65
        ];
        let out = structural().run(entities);
        assert_eq!(out.kept().len(), 1);
        assert_eq!(*out.kept()[0].label(), LabelRef::new("payment_card"));
        assert_eq!(out.dropped().len(), 1);
    }

    /// A near-coincident cross-label overlap is a true conflict: the winner
    /// survives and records the loser by id; the loser is dropped in the
    /// winner's favor.
    #[test]
    fn structural_resolves_true_conflict() {
        let (person, org) = (
            entity("PERSON_NAME", 0..12, 0.9),
            entity("ORGANIZATION", 2..14, 0.7),
        );
        let (person_id, org_id) = (person.id(), org.id());
        let layer = structural();
        let out = layer.run(vec![person, org]);

        let [winner] = out.kept() else {
            panic!("one survivor: {:?}", out.kept());
        };
        assert_eq!(winner.id(), person_id);
        assert!(has_kind(winner, |k| matches!(
            k,
            AuditKind::Resolved(c) if c.competing_id == org_id
                && c.competing_label == LabelRef::new("ORGANIZATION")
                && c.resolved_by == layer.id()
        )));

        let [(loser, winner)] = out.dropped() else {
            panic!("one drop: {:?}", out.dropped());
        };
        assert_eq!(loser.id(), org_id);
        assert_eq!(*winner, Some(person_id));
    }

    /// A winner that later loses to a third entity still records its own win:
    /// each loser names the entity that beat it, so the chain reads whole.
    #[test]
    fn exclusive_chains_each_loser_to_its_winner() {
        let (strong, middle, weak) = (
            entity("IBAN", 0..12, 0.9),
            entity("PERSON_NAME", 0..12, 0.6),
            entity("ORGANIZATION", 0..12, 0.3),
        );
        let ids = (strong.id(), middle.id(), weak.id());
        // `middle` meets `weak` first, then loses to `strong`.
        let out = ReconcileLayer::exclusive().run(vec![middle, weak, strong]);

        assert_eq!(out.kept().len(), 1);
        assert_eq!(out.kept()[0].id(), ids.0);
        let dropped = |id| {
            out.dropped()
                .iter()
                .find(|(entity, _)| entity.id() == id)
                .expect("dropped")
        };
        assert_eq!(dropped(ids.1).1, Some(ids.0));
        assert_eq!(dropped(ids.2).1, Some(ids.1));
        assert!(has_kind(&dropped(ids.1).0, |k| matches!(
            k,
            AuditKind::Resolved(c) if c.competing_id == ids.2
        )));
    }

    /// With `OnConflict::Contest`, a true conflict keeps BOTH, each flagged
    /// contested against the other, nothing dropped.
    #[test]
    fn structural_contests_when_reviewing() {
        let entities = vec![
            entity("PERSON_NAME", 0..12, 0.9),
            entity("ORGANIZATION", 2..14, 0.7),
        ];
        let layer = ReconcileLayer::new(Structural::standard().reviewing());
        let out = layer.run(entities);
        assert_eq!(out.kept().len(), 2, "contest keeps both");
        assert!(out.dropped().is_empty());
        let (a, b) = (&out.kept()[0], &out.kept()[1]);
        for (entity, other) in [(a, b), (b, a)] {
            assert!(has_kind(entity, |k| matches!(
                k,
                AuditKind::Contested(c) if c.competing_id == other.id()
                    && c.flagged_by == layer.id()
            )));
        }
    }

    /// Two labels on exactly the same span are a true conflict, not a nesting:
    /// the more confident wins whichever comes first, whether the two are
    /// further apart than the nesting margin or within it.
    #[test]
    fn structural_resolves_identical_spans_in_either_order() {
        for (weak, strong) in [(0.5, 0.9), (0.8, 0.9)] {
            let person = || entity("PERSON_NAME", 0..12, weak);
            let org = || entity("ORGANIZATION", 0..12, strong);
            for entities in [vec![person(), org()], vec![org(), person()]] {
                let out = structural().run(entities);
                assert_eq!(out.kept().len(), 1);
                assert_eq!(*out.kept()[0].label(), LabelRef::new("ORGANIZATION"));
                assert!(has_kind(&out.kept()[0], |k| matches!(
                    k,
                    AuditKind::Resolved(c) if c.competing_label == LabelRef::new("PERSON_NAME")
                )));
            }
        }
    }

    /// Reviewing contests two labels on exactly the same span, however close
    /// their confidences.
    #[test]
    fn structural_contests_identical_spans_when_reviewing() {
        let entities = vec![
            entity("PERSON_NAME", 0..12, 0.8),
            entity("ORGANIZATION", 0..12, 0.7),
        ];
        let layer = ReconcileLayer::new(Structural::standard().reviewing());
        let out = layer.run(entities);
        assert_eq!(out.kept().len(), 2);
        for e in out.kept() {
            assert!(has_kind(e, |k| matches!(k, AuditKind::Contested(_))));
        }
    }

    /// `Permissive` keeps every cross-label overlap untouched (Presidio-style).
    #[test]
    fn permissive_keeps_everything() {
        let entities = vec![
            entity("PERSON_NAME", 0..12, 0.9),
            entity("ORGANIZATION", 2..14, 0.7),
        ];
        let out = ReconcileLayer::permissive().run(entities);
        assert_eq!(out.kept().len(), 2);
        assert!(out.dropped().is_empty());
    }
}
