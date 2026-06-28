//! The [`Permissive`] reconciler.

use elide_core::entity::Entity;
use elide_core::modality::Modality;

use super::{Disposition, Reconciler};

/// The permissive reconciler: every grouped pair keeps both, unresolved.
///
/// Presidio-style — no cross-label resolution at detection; overlaps survive
/// and are left to the human edit step and the anonymizer's overlap-safe
/// apply. Use [`contesting`] to flag them for review instead of leaving them
/// silent.
///
/// [`contesting`]: Permissive::contesting
#[derive(Debug, Clone, Copy, Default)]
pub struct Permissive {
    /// Whether survivors are flagged contested (for review) or kept silently.
    contest: bool,
}

impl Permissive {
    /// Keep every overlap, unflagged.
    pub fn new() -> Self {
        Self { contest: false }
    }

    /// Keep every overlap, but flag each pair contested for the human edit
    /// step.
    pub fn contesting() -> Self {
        Self { contest: true }
    }
}

impl<M: Modality> Reconciler<M> for Permissive {
    fn decide(&self, _a: &Entity<M>, _b: &Entity<M>) -> Disposition {
        if self.contest {
            Disposition::Contest
        } else {
            Disposition::KeepBoth
        }
    }

    fn name(&self) -> &'static str {
        "permissive"
    }
}
