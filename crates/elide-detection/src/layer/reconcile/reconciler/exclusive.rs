//! The [`Exclusive`] reconciler.

use elide_core::entity::Entity;
use elide_core::modality::Modality;

use super::super::tiebreaker::{HighestConfidence, LongestSpan, Tiebreaker};
use super::{Disposition, Reconciler, Winner};

/// The aggressive reconciler: one finding per span.
///
/// Every grouped pair is a conflict, resolved by `tiebreaker` — never keeps
/// both. For callers who want a strict, mutually-exclusive output (no nesting,
/// no co-existing overlaps).
#[derive(Debug, Clone, Copy, Default)]
pub struct Exclusive<T = HighestConfidence> {
    /// How the winner is chosen.
    pub tiebreaker: T,
}

impl<T> Exclusive<T> {
    /// An exclusive reconciler using `tiebreaker`.
    pub fn new(tiebreaker: T) -> Self {
        Self { tiebreaker }
    }
}

impl Exclusive<HighestConfidence> {
    /// An exclusive reconciler keeping the higher-confidence finding (the
    /// default).
    pub fn highest_confidence() -> Self {
        Self::new(HighestConfidence)
    }
}

impl Exclusive<LongestSpan> {
    /// An exclusive reconciler keeping the larger-span finding.
    pub fn longest_span() -> Self {
        Self::new(LongestSpan)
    }
}

impl<M, T> Reconciler<M> for Exclusive<T>
where
    M: Modality,
    T: Tiebreaker<M>,
{
    fn decide(&self, a: &Entity<M>, b: &Entity<M>) -> Disposition {
        Disposition::Resolve {
            winner: if self.tiebreaker.keeps_first(a, b) {
                Winner::First
            } else {
                Winner::Second
            },
        }
    }

    fn name(&self) -> &'static str {
        "exclusive"
    }
}
