//! [`FuseLayer`]: fuse each cluster of the same finding into one entity.

use std::mem;

use elide_core::entity::Entity;
use elide_core::entity::audit::AuditEvent;
use elide_core::modality::{Modality, ModalityLocation};

use super::group::{GroupPredicate, SameLabel};
use super::scoring::{MaxConfidence, NoisyOrConfidence, Scoring};
use super::{Cluster, cluster};
use crate::layer::{Layer, LayerOutput};

/// The fusion stage: every cluster becomes one entity.
///
/// A [`GroupPredicate`] (by default [`SameLabel`]: same label, overlapping)
/// decides which findings are the same finding; each cluster of them fuses into
/// its highest-confidence member, which takes the union of every member's span
/// and the confidence the [`Scoring`] pools over all of them. Its audit trail
/// joins every member's: one fusion event whose parents are each member's trail
/// head. A lone entity passes through unchanged.
///
/// Pooling covers the whole cluster at once, so a scoring that accumulates
/// evidence ([`NoisyOrConfidence`]) counts every witness, whatever order they
/// arrived in.
#[derive(Debug, Clone, Copy, Default)]
pub struct FuseLayer<G = SameLabel, S = MaxConfidence> {
    group: G,
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
    fn apply(&self, entities: Vec<Entity<M>>) -> LayerOutput<M> {
        let clusters = cluster(&self.group, &entities);
        let mut slots: Vec<Option<Entity<M>>> = entities.into_iter().map(Some).collect();
        let mut dropped = Vec::new();
        for members in clusters {
            if members.len() > 1 {
                dropped.extend(self.fuse(&mut slots, &members));
            }
        }
        LayerOutput::split(slots.into_iter().flatten().collect(), dropped)
    }
}

impl<G, S: Scoring> FuseLayer<G, S> {
    /// Fuse `members` into the highest-confidence one (ties to the earliest),
    /// left in its slot; returns the members it absorbed, emptied of their
    /// trails.
    ///
    /// A member whose location cannot be united with the fused one (a custom
    /// grouping that clusters findings on different pages, say) is left in its
    /// own slot, a separate entity: fusion merges only what it can locate as
    /// one, and never drops a finding it cannot cover.
    fn fuse<M: Modality>(
        &self,
        slots: &mut [Option<Entity<M>>],
        members: &Cluster,
    ) -> Vec<Entity<M>> {
        let mut base_index = members[0];
        for &index in &members[1..] {
            let candidate = slots[index].as_ref().map(|e| e.confidence);
            let base = slots[base_index].as_ref().map(|e| e.confidence);
            if candidate > base || (candidate == base && index < base_index) {
                base_index = index;
            }
        }
        let mut base = slots[base_index].take().expect("each member is fused once");

        let mut absorbed: Vec<Entity<M>> = Vec::with_capacity(members.len() - 1);
        for &index in members.iter().filter(|&&index| index != base_index) {
            let other = slots[index].take().expect("each member is fused once");
            match base.location.union(&other.location) {
                Some(union) => {
                    base.location = union;
                    absorbed.push(other);
                }
                None => slots[index] = Some(other),
            }
        }
        if absorbed.is_empty() {
            slots[base_index] = Some(base);
            return absorbed;
        }

        let confidence = absorbed.iter().fold(base.confidence, |pooled, other| {
            self.scoring.score(pooled, other.confidence)
        });
        // The fusion event joins every member trail that has a head; a member
        // with no events yet (built without provenance) contributes none.
        let mut heads = Vec::with_capacity(absorbed.len() + 1);
        if !base.audit.is_empty() {
            heads.push(base.audit.head_hash());
        }
        for other in &mut absorbed {
            if !other.audit.is_empty() {
                heads.push(other.audit.head_hash());
            }
            base.audit.absorb(mem::take(&mut other.audit).into_events());
        }
        base.confidence = confidence;
        base.audit.record_fusion(
            AuditEvent::deduplication(self.scoring.name(), confidence),
            heads,
        );
        slots[base_index] = Some(base);
        absorbed
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::audit::{AuditEvent, AuditKind, PatternRule};
    use elide_core::entity::{Entity, LabelRef};
    use elide_core::modality::text::{Text, TextLocation};
    use elide_core::primitive::{Confidence, UnitBoundingBox};

    use super::*;

    fn entity(label: &str, start: usize, end: usize, conf: f32) -> Entity<Text> {
        let loc = TextLocation::inline(start..end);
        let confidence = Confidence::new(conf).unwrap();
        let event = AuditEvent::pattern("t", confidence, loc.clone(), PatternRule::default());
        Entity::new(LabelRef::new(label), loc).with_event(event)
    }

    /// Same-label overlapping findings fuse into one entity spanning their
    /// union, with the pooled confidence.
    #[test]
    fn fuses_same_label_into_the_union() {
        let entities = vec![entity("EMAIL", 0, 10, 0.9), entity("EMAIL", 6, 25, 0.6)];
        let out = FuseLayer::max().apply(entities);
        assert_eq!(out.kept.len(), 1);
        let fused = &out.kept[0];
        let range = fused.location.range().unwrap();
        assert_eq!((range.start, range.end), (0, 25));
        assert_eq!(fused.label, LabelRef::new("EMAIL"));
        assert_eq!(fused.confidence, Confidence::new(0.9).unwrap());
        assert_eq!(out.dropped.len(), 1);
    }

    /// `NoisyOrConfidence` accumulates: two agreeing 0.6 witnesses → 0.84.
    #[test]
    fn noisy_or_accumulates() {
        let entities = vec![entity("EMAIL", 0, 10, 0.6), entity("EMAIL", 0, 10, 0.6)];
        let out = FuseLayer::noisy_or().apply(entities);
        assert_eq!(out.kept.len(), 1);
        assert!((out.kept[0].confidence.get() - 0.84).abs() < 1e-5);
    }

    /// `NoisyOrConfidence` pools every member of a cluster: three agreeing 0.6
    /// witnesses → 1 − 0.4³ = 0.936.
    #[test]
    fn noisy_or_pools_a_cluster_of_three() {
        let entities = vec![
            entity("EMAIL", 0, 10, 0.6),
            entity("EMAIL", 0, 10, 0.6),
            entity("EMAIL", 0, 10, 0.6),
        ];
        let out = FuseLayer::noisy_or().apply(entities);
        assert_eq!(out.kept.len(), 1);
        assert!(
            (out.kept[0].confidence.get() - 0.936).abs() < 1e-5,
            "got {}",
            out.kept[0].confidence.get()
        );
    }

    /// A chain that only links through its middle member (A–B, B–C overlap, A–C
    /// do not) still fuses into one entity over the whole span.
    #[test]
    fn a_chained_cluster_fuses_into_one() {
        let entities = vec![
            entity("EMAIL", 0, 10, 0.9),
            entity("EMAIL", 8, 18, 0.8),
            entity("EMAIL", 16, 26, 0.7),
        ];
        let out = FuseLayer::max().apply(entities);
        assert_eq!(out.kept.len(), 1);
        let range = out.kept[0].location.range().unwrap();
        assert_eq!((range.start, range.end), (0, 26));
    }

    /// A three-way fusion is one event joining all three trails, not a chain of
    /// pairwise fusions, and the joined trail still verifies.
    #[test]
    fn a_fusion_joins_every_member_trail_in_one_event() {
        let entities = vec![
            entity("EMAIL", 0, 10, 0.9),
            entity("EMAIL", 0, 10, 0.8),
            entity("EMAIL", 0, 10, 0.7),
        ];
        let out = FuseLayer::max().apply(entities);
        let audit = &out.kept[0].audit;
        let fusions: Vec<_> = audit
            .events()
            .iter()
            .filter(|event| matches!(event.kind, AuditKind::Deduplication(_)))
            .collect();
        assert_eq!(fusions.len(), 1);
        assert_eq!(fusions[0].parents().len(), 3);
        assert!(audit.verify().is_ok());
    }

    /// Members built without provenance (empty trails) fuse into a trail that
    /// still verifies: the fusion event names no parent it does not have.
    #[test]
    fn fusing_entities_with_no_trail_verifies() {
        let entities: Vec<Entity<Text>> = vec![
            Entity::new(LabelRef::new("EMAIL"), TextLocation::inline(0..10)),
            Entity::new(LabelRef::new("EMAIL"), TextLocation::inline(5..15)),
        ];
        let out = FuseLayer::max().apply(entities);
        assert_eq!(out.kept.len(), 1);
        assert!(out.kept[0].audit.verify().is_ok());
    }

    /// Groups every finding of a label, wherever it is.
    struct AnyWhere;

    impl GroupPredicate<Text> for AnyWhere {
        type Bucket = LabelRef;

        fn bucket(&self, entity: &Entity<Text>) -> LabelRef {
            entity.label.clone()
        }

        fn is_grouped(&self, _a: &Entity<Text>, _b: &Entity<Text>) -> bool {
            true
        }
    }

    /// A grouping can cluster findings whose locations cannot be united (two
    /// pages); fusion keeps each as its own entity rather than dropping one.
    #[test]
    fn a_member_that_cannot_be_united_stays_separate() {
        let on_page = |page: u32| {
            let location =
                TextLocation::rendered(0..5, page, [UnitBoundingBox::new(0.0, 0.0, 1.0, 1.0)]);
            let confidence = Confidence::new(0.9).unwrap();
            let event =
                AuditEvent::pattern("t", confidence, location.clone(), PatternRule::default());
            Entity::new(LabelRef::new("EMAIL"), location).with_event(event)
        };
        let out = FuseLayer::max()
            .with_grouping(AnyWhere)
            .apply(vec![on_page(1), on_page(2)]);
        assert_eq!(out.kept.len(), 2, "both pages' findings survive");
        assert!(out.dropped.is_empty());
    }

    /// Different labels never group under `SameLabel`, so fusion leaves a
    /// cross-label overlap alone.
    #[test]
    fn fusion_ignores_different_labels() {
        let entities = vec![entity("EMAIL", 0, 10, 0.9), entity("PHONE", 0, 10, 0.8)];
        let out = FuseLayer::max().apply(entities);
        assert_eq!(out.kept.len(), 2);
        assert!(out.dropped.is_empty());
    }
}
