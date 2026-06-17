//! The [`ConflictResolution`] trait and the shipped strategies.

use veil_core::entity::Entity;
use veil_core::modality::{Modality, ModalityLocation};

/// Decides which of two overlapping, differently-labelled entities to
/// keep.
///
/// A *type*, used as the `R` parameter of [`ResolveLayer`]. The crate
/// ships [`HighestConfidence`] and [`LongestSpan`]; a consumer can add
/// their own arbitration.
///
/// [`ResolveLayer`]: super::ResolveLayer
pub trait ConflictResolution<M: Modality>: Send + Sync {
    /// Whether `a` should be kept over `b`. `true` keeps `a` and drops
    /// `b`; `false` the reverse.
    fn keeps_first(&self, a: &Entity<M>, b: &Entity<M>) -> bool;
}

/// Keep the higher-confidence entity (ties keep the first).
#[derive(Debug, Clone, Copy, Default)]
pub struct HighestConfidence;

impl<M: Modality> ConflictResolution<M> for HighestConfidence {
    fn keeps_first(&self, a: &Entity<M>, b: &Entity<M>) -> bool {
        a.confidence >= b.confidence
    }
}

/// Keep the entity covering the larger span — the more specific match
/// (ties keep the first).
#[derive(Debug, Clone, Copy, Default)]
pub struct LongestSpan;

impl<M: Modality> ConflictResolution<M> for LongestSpan {
    fn keeps_first(&self, a: &Entity<M>, b: &Entity<M>) -> bool {
        a.location.span_cmp(&b.location) != std::cmp::Ordering::Less
    }
}
