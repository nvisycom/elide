//! The [`Tiebreaker`] trait and the shipped tiebreakers.

use std::cmp::Ordering;

use elide_core::entity::Entity;
use elide_core::modality::{Modality, ModalityLocation};

/// Picks the winner between two entities judged to conflict.
///
/// A *type*, the `T` parameter of a geometry-aware reconciler ([`Structural`],
/// [`Exclusive`]). The crate ships [`HighestConfidence`] and [`LongestSpan`]; a
/// consumer can supply their own (recognizer priority, value-aware
/// arbitration, …).
///
/// [`Structural`]: super::reconciler::Structural
/// [`Exclusive`]: super::reconciler::Exclusive
pub trait Tiebreaker<M: Modality>: Send + Sync {
    /// Whether `a` should be kept over `b`. `true` keeps `a`, drops `b`.
    fn keeps_first(&self, a: &Entity<M>, b: &Entity<M>) -> bool;
}

/// Keep the higher-confidence entity (ties keep the first).
#[derive(Debug, Clone, Copy, Default)]
pub struct HighestConfidence;

impl<M: Modality> Tiebreaker<M> for HighestConfidence {
    fn keeps_first(&self, a: &Entity<M>, b: &Entity<M>) -> bool {
        a.confidence >= b.confidence
    }
}

/// Keep the entity covering the larger span — the more specific match (ties
/// keep the first).
#[derive(Debug, Clone, Copy, Default)]
pub struct LongestSpan;

impl<M: Modality> Tiebreaker<M> for LongestSpan {
    fn keeps_first(&self, a: &Entity<M>, b: &Entity<M>) -> bool {
        a.location.span_cmp(&b.location) != Ordering::Less
    }
}
