//! [`FuseLayer`]: fuse each cluster of the same finding into one entity.

use elide_core::Component;
use elide_core::entity::Entity;
use elide_core::entity::audit::Fused;
use elide_core::modality::{Modality, ModalityLocation};
use elide_core::recognition::Context;

use super::group::{GroupPredicate, SameLabel};
use super::scoring::{MaxConfidence, NoisyOrConfidence, Scoring};
use super::{Cluster, cluster};
use crate::layer::{Layer, LayerOutput};

/// The fusion stage: each cluster of the same finding becomes one entity.
///
/// A [`GroupPredicate`] (by default [`SameLabel`]: same label, overlapping)
/// decides which findings are the same finding; each cluster of them fuses into
/// its highest-confidence member, which takes the union of every member's span
/// and the confidence the [`Scoring`] pools over all of them. Its audit trail
/// joins every member's: one fusion event whose parents are each member's trail
/// head. A lone entity passes through unchanged.
///
/// Fusion drops nothing: each absorbed member, its whole trail, lives on in
/// the fused entity. The layer is identified by its [`Scoring`].
///
/// Pooling covers the whole cluster at once, so a scoring that accumulates
/// evidence ([`NoisyOrConfidence`]) counts every witness, whatever order they
/// arrived in.
#[derive(Debug, Clone, Copy, Default, Component)]
pub struct FuseLayer<G = SameLabel, S = MaxConfidence> {
    group: G,
    #[component(delegate)]
    scoring: S,
}

impl<S> FuseLayer<SameLabel, S> {
    /// A fusion layer over same-label overlaps, pooling confidence with
    /// `scoring`.
    pub fn new(scoring: S) -> Self {
        Self {
            group: SameLabel,
            scoring,
        }
    }
}

impl FuseLayer<SameLabel, MaxConfidence> {
    /// Fuse same-label overlaps, keeping the highest confidence (the default).
    pub fn max() -> Self {
        Self::new(MaxConfidence)
    }
}

impl FuseLayer<SameLabel, NoisyOrConfidence> {
    /// Fuse same-label overlaps, accumulating agreeing findings' evidence
    /// (`1 − ∏(1 − pᵢ)`).
    pub fn noisy_or() -> Self {
        Self::new(NoisyOrConfidence)
    }
}

impl<G, S> FuseLayer<G, S> {
    /// Cluster by `group` instead: which findings count as the same finding.
    #[must_use]
    pub fn with_grouping<G2>(self, group: G2) -> FuseLayer<G2, S> {
        FuseLayer {
            group,
            scoring: self.scoring,
        }
    }
}

impl<M, G, S> Layer<M> for FuseLayer<G, S>
where
    M: Modality,
    G: GroupPredicate<M>,
    S: Scoring,
{
    fn apply(&self, entities: Vec<Entity<M>>, _ctx: &Context<'_>) -> LayerOutput<M> {
        let clusters = cluster(&self.group, &entities);
        let mut slots: Vec<Option<Entity<M>>> = entities.into_iter().map(Some).collect();
        for members in clusters {
            // Fuse in rounds: each round fuses the members that can be located
            // as one with its strongest member, and the rest fuse among
            // themselves in the next. Each round settles at least its base.
            let mut remaining = members;
            while remaining.len() > 1 {
                remaining = self.fuse(&mut slots, &remaining);
            }
        }
        let mut output = LayerOutput::new();
        for entity in slots.into_iter().flatten() {
            output.keep(entity);
        }
        output
    }
}

impl<G, S: Scoring> FuseLayer<G, S> {
    /// Fuse `members` into the highest-confidence one (ties to the earliest),
    /// left in its slot; returns the indices of the ones it could not unite.
    ///
    /// A member whose location cannot be united with the fused one (a custom
    /// grouping that clusters findings on different pages, say) is left in its
    /// own slot for the next round: fusion merges only what it can locate as
    /// one, and never drops a finding it cannot cover.
    fn fuse<M: Modality>(&self, slots: &mut [Option<Entity<M>>], members: &Cluster) -> Cluster {
        let mut base_index = members[0];
        for &index in &members[1..] {
            let candidate = slots[index].as_ref().map(|e| e.confidence());
            let base = slots[base_index].as_ref().map(|e| e.confidence());
            if candidate > base || (candidate == base && index < base_index) {
                base_index = index;
            }
        }
        let mut base = slots[base_index].take().expect("each member is fused once");

        let mut absorbed: Vec<Entity<M>> = Vec::with_capacity(members.len() - 1);
        let mut rest = Cluster::new();
        let mut location = base.location().clone();
        for &index in members.iter().filter(|&&index| index != base_index) {
            let other = slots[index].take().expect("each member is fused once");
            match location.union(other.location()) {
                Some(union) => {
                    location = union;
                    absorbed.push(other);
                }
                None => {
                    slots[index] = Some(other);
                    rest.push(index);
                }
            }
        }
        if absorbed.is_empty() {
            slots[base_index] = Some(base);
            return rest;
        }

        let confidence = absorbed.iter().fold(base.confidence(), |pooled, other| {
            self.scoring.score(pooled, other.confidence())
        });
        base.fuse(
            &absorbed,
            Fused::new(self.scoring.id(), location),
            confidence,
        );
        slots[base_index] = Some(base);
        rest
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use elide_core::entity::audit::{AuditKind, Pattern, PatternRule};
    use elide_core::entity::{Birth, Entity, LabelRef};
    use elide_core::modality::text::{Text, TextLocation};
    use elide_core::primitive::{Confidence, UnitBoundingBox};

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

    /// Same-label overlapping findings fuse into one entity spanning their
    /// union, with the pooled confidence. Nothing is dropped: the absorbed
    /// member's trail lives on in the fused entity's.
    #[test]
    fn fuses_same_label_into_the_union() {
        let absorbed = entity("EMAIL", 6..25, 0.6);
        let entities = vec![entity("EMAIL", 0..10, 0.9), absorbed.clone()];
        let out = FuseLayer::max().run(entities);
        assert_eq!(out.kept().len(), 1);
        assert!(out.dropped().is_empty(), "fusion drops nothing");
        let fused = &out.kept()[0];
        let range = fused.location().range().unwrap();
        assert_eq!((range.start, range.end), (0, 25));
        assert_eq!(*fused.label(), LabelRef::new("EMAIL"));
        assert_eq!(fused.confidence(), Confidence::new(0.9).unwrap());
        assert!(
            fused
                .audit()
                .events()
                .iter()
                .any(|event| event.hash() == absorbed.audit().tip()),
            "the absorbed member's trail is in the fused entity's"
        );
        assert!(has_kind(fused, |kind| matches!(
            kind,
            AuditKind::Fused(fused) if fused.strategy == MaxConfidence.id()
        )));
    }

    /// `NoisyOrConfidence` accumulates: two agreeing 0.6 witnesses → 0.84.
    #[test]
    fn noisy_or_accumulates() {
        let entities = vec![entity("EMAIL", 0..10, 0.6), entity("EMAIL", 0..10, 0.6)];
        let out = FuseLayer::noisy_or().run(entities);
        assert_eq!(out.kept().len(), 1);
        assert!((out.kept()[0].confidence().get() - 0.84).abs() < 1e-5);
    }

    /// `NoisyOrConfidence` pools every member of a cluster: three agreeing 0.6
    /// witnesses → 1 − 0.4³ = 0.936.
    #[test]
    fn noisy_or_pools_a_cluster_of_three() {
        let entities = vec![
            entity("EMAIL", 0..10, 0.6),
            entity("EMAIL", 0..10, 0.6),
            entity("EMAIL", 0..10, 0.6),
        ];
        let out = FuseLayer::noisy_or().run(entities);
        assert_eq!(out.kept().len(), 1);
        assert!(
            (out.kept()[0].confidence().get() - 0.936).abs() < 1e-5,
            "got {}",
            out.kept()[0].confidence().get()
        );
    }

    /// A chain that only links through its middle member (A–B, B–C overlap, A–C
    /// do not) still fuses into one entity over the whole span.
    #[test]
    fn a_chained_cluster_fuses_into_one() {
        let entities = vec![
            entity("EMAIL", 0..10, 0.9),
            entity("EMAIL", 8..18, 0.8),
            entity("EMAIL", 16..26, 0.7),
        ];
        let out = FuseLayer::max().run(entities);
        assert_eq!(out.kept().len(), 1);
        let range = out.kept()[0].location().range().unwrap();
        assert_eq!((range.start, range.end), (0, 26));
    }

    /// A three-way fusion is one event joining all three trails, not a chain of
    /// pairwise fusions, and the joined trail still verifies.
    #[test]
    fn a_fusion_joins_every_member_trail_in_one_event() {
        let entities = vec![
            entity("EMAIL", 0..10, 0.9),
            entity("EMAIL", 0..10, 0.8),
            entity("EMAIL", 0..10, 0.7),
        ];
        let out = FuseLayer::max().run(entities);
        let audit = out.kept()[0].audit();
        let fusions: Vec<_> = audit
            .events()
            .iter()
            .filter(|event| matches!(event.kind, AuditKind::Fused(_)))
            .collect();
        assert_eq!(fusions.len(), 1);
        assert_eq!(fusions[0].parents().len(), 3);
        assert!(audit.verify().is_ok());
    }

    /// Groups every finding of a label, wherever it is.
    struct AnyWhere;

    impl GroupPredicate<Text> for AnyWhere {
        type Bucket = LabelRef;

        fn bucket(&self, entity: &Entity<Text>) -> LabelRef {
            entity.label().clone()
        }

        fn is_grouped(&self, _a: &Entity<Text>, _b: &Entity<Text>) -> bool {
            true
        }
    }

    /// A grouping can cluster findings whose locations cannot be united (two
    /// pages); fusion keeps each page's findings apart rather than dropping
    /// one, and still fuses the findings that share a page.
    #[test]
    fn members_that_cannot_be_united_fuse_by_location() {
        let on_page = |page: u32, conf: f32| {
            let location =
                TextLocation::rendered(0..5, page, [UnitBoundingBox::new(0.0, 0.0, 1.0, 1.0)]);
            let confidence = Confidence::new(conf).unwrap();
            Entity::new(Birth::new(
                "t",
                confidence,
                Pattern::new(LabelRef::new("EMAIL"), location, PatternRule::default()),
            ))
        };
        let out = FuseLayer::noisy_or().with_grouping(AnyWhere).run(vec![
            on_page(1, 0.9),
            on_page(2, 0.6),
            on_page(2, 0.6),
        ]);
        assert_eq!(
            out.kept().len(),
            2,
            "one entity per page, the page-2 pair fused"
        );
        assert!(out.dropped().is_empty());
        let page_two = out
            .kept()
            .iter()
            .find(|entity| entity.location().page() == Some(2))
            .expect("page 2 survives");
        assert!((page_two.confidence().get() - 0.84).abs() < 1e-5);
    }

    /// Different labels never group under `SameLabel`, so fusion leaves a
    /// cross-label overlap alone.
    #[test]
    fn fusion_ignores_different_labels() {
        let entities = vec![entity("EMAIL", 0..10, 0.9), entity("PHONE", 0..10, 0.8)];
        let out = FuseLayer::max().run(entities);
        assert_eq!(out.kept().len(), 2);
        assert!(out.dropped().is_empty());
    }
}
