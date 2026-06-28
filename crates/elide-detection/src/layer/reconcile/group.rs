//! The [`GroupPredicate`] trait and the shipped groupings.

use std::hash::Hash;

use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::{Modality, ModalityLocation};

/// Decides which entities form a cluster — the *which* axis of
/// reconciliation.
///
/// A *type*, the `G` parameter of [`ReconcileLayer`]. Grouping is two
/// dimensions: a coarse [`bucket`] (a cheap `O(n)` equality partition) and a
/// fine [`is_grouped`] (a pairwise refinement within a bucket). The crate
/// ships [`SameLabel`] (same label, overlapping — the fusion default) and
/// [`CrossLabel`] (different label, overlapping — the conflict default);
/// a consumer can supply a value-aware or looser grouping.
///
/// **Law:** `is_grouped(a, b)` implies `bucket(a) == bucket(b)`. The layer
/// clusters within each bucket, so two entities the predicate considers
/// grouped must share a bucket or they will never be compared. A grouping
/// that can't cheaply partition uses `Bucket = ()` — one bucket, every pair
/// compared.
///
/// [`ReconcileLayer`]: super::ReconcileLayer
/// [`bucket`]: GroupPredicate::bucket
/// [`is_grouped`]: GroupPredicate::is_grouped
pub trait GroupPredicate<M: Modality>: Send + Sync {
    /// The coarse partition: entities with *different* buckets are never
    /// grouped, so the layer clusters within each bucket instead of scanning
    /// all pairs.
    type Bucket: Hash + Eq;

    /// The bucket `entity` falls into. Two entities in different buckets can
    /// never be grouped (see the trait law).
    fn bucket(&self, entity: &Entity<M>) -> Self::Bucket;

    /// Whether `a` and `b` belong to the same cluster. Only ever called for
    /// entities sharing a bucket.
    fn is_grouped(&self, a: &Entity<M>, b: &Entity<M>) -> bool;
}

/// Group entities that share a label and whose locations overlap.
///
/// The fusion grouping: co-located findings of the same label are the same
/// finding. Buckets by label, so clustering is one pass per label.
#[derive(Debug, Clone, Copy, Default)]
pub struct SameLabel;

impl<M: Modality> GroupPredicate<M> for SameLabel {
    type Bucket = LabelRef;

    fn bucket(&self, entity: &Entity<M>) -> LabelRef {
        entity.label.clone()
    }

    fn is_grouped(&self, a: &Entity<M>, b: &Entity<M>) -> bool {
        // The bucket already guarantees equal labels; only overlap remains.
        a.location.overlaps(&b.location)
    }
}

/// Group entities of *different* labels whose locations overlap.
///
/// The conflict grouping: a cross-label overlap is a candidate to arbitrate.
/// Grouped entities differ in label, so no label-based bucket can hold them
/// together — `Bucket = ()` places every entity in one bucket (every pair
/// compared). Cross-label tangles are small, so the `O(n²)` within-bucket
/// scan is cheap in practice.
#[derive(Debug, Clone, Copy, Default)]
pub struct CrossLabel;

impl<M: Modality> GroupPredicate<M> for CrossLabel {
    type Bucket = ();

    fn bucket(&self, _entity: &Entity<M>) {}

    fn is_grouped(&self, a: &Entity<M>, b: &Entity<M>) -> bool {
        a.label != b.label && a.location.overlaps(&b.location)
    }
}
