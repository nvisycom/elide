//! The [`GroupPredicate`] trait and the default grouping.

use elide_core::entity::Entity;
use elide_core::modality::{Modality, ModalityLocation};

/// Decides whether two entities are the same finding and should fuse.
///
/// A *type*, used as the `G` parameter of [`FuseLayer`]. The default,
/// [`SameLabelOverlap`], groups entities with the same label whose
/// locations overlap — the common case; a consumer can supply a
/// value-aware or looser predicate.
///
/// [`FuseLayer`]: super::FuseLayer
pub trait GroupPredicate<M: Modality>: Send + Sync {
    /// Whether `a` and `b` denote the same finding.
    fn same(&self, a: &Entity<M>, b: &Entity<M>) -> bool;
}

/// Group entities sharing a label whose locations overlap.
#[derive(Debug, Clone, Copy, Default)]
pub struct SameLabelOverlap;

impl<M: Modality> GroupPredicate<M> for SameLabelOverlap {
    fn same(&self, a: &Entity<M>, b: &Entity<M>) -> bool {
        a.label == b.label && a.location.overlaps(&b.location)
    }
}
