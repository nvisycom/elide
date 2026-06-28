//! The [`Exclusive`] reconciler.

use elide_core::entity::Entity;
use elide_core::modality::Modality;

use super::super::tiebreaker::{HighestConfidence, Tiebreaker};
use super::{Disposition, Reconciler, Winner};

/// The aggressive reconciler: every grouped pair is a conflict, resolved by
/// `tiebreaker`. Never keeps both — one finding per span.
///
/// For callers who want a strict, mutually-exclusive output (no nesting, no
/// co-existing overlaps).
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
