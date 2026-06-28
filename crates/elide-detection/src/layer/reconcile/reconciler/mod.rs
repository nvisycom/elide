//! The [`Reconciler`] trait — the *what to do* axis of reconciliation — its
//! [`Disposition`] outcome, and the shipped reconcilers.
//!
//! A reconciler decides each grouped pair's fate. The crate ships [`Merging`]
//! (combine same-label findings), [`Structural`] (geometry-aware cross-label
//! handling), [`Exclusive`] (one finding per span), and [`Permissive`] (keep
//! every overlap).

mod exclusive;
mod merging;
mod permissive;
mod structural;

use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::primitive::Confidence;

pub use self::exclusive::Exclusive;
pub use self::merging::Merging;
pub use self::permissive::Permissive;
pub use self::structural::{OnConflict, Structural};

/// Which of two entities wins a [`Resolve`][Disposition::Resolve].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Winner {
    /// The first entity wins; the second is dropped.
    First,
    /// The second entity wins; the first is dropped.
    Second,
}

/// What to do with two grouped entities — the *what* axis of reconciliation.
///
/// The single outcome a [`Reconciler`] returns for a pair. The layer folds
/// these over a cluster: drops take effect first, then contests flag, then
/// merges union the survivors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Disposition {
    /// Combine the pair into one entity: union their spans and set the given
    /// pooled `confidence`. The reconciler computes the pool, so the layer
    /// stays free of scoring policy.
    Merge {
        /// The pooled confidence the merged entity takes.
        confidence: Confidence,
    },
    /// Both entities survive, unflagged — a legitimate co-existence (a
    /// nesting, or two distinct findings that merely touch).
    KeepBoth,
    /// Both entities survive, each flagged *contested* against the other, for
    /// a human to resolve at the edit step. A genuine disagreement the
    /// machine declines to settle.
    Contest,
    /// One entity wins; the loser is dropped and recorded on the winner's
    /// provenance.
    Resolve {
        /// Which entity wins.
        winner: Winner,
    },
}

/// Decides what happens to two grouped entities — the `R` parameter of
/// [`ReconcileLayer`].
///
/// A *type*, not a stringly-tagged enum. Pairwise: the layer owns clustering
/// and folds the per-pair [`Disposition`]s. The crate ships [`Merging`]
/// (always merge, pooling confidence), [`Structural`] (geometry-aware:
/// nesting keeps both, subsumed junk resolves, true conflict resolves or
/// contests), [`Exclusive`] (always resolve), and [`Permissive`] (always keep
/// both). A consumer can supply their own.
///
/// [`ReconcileLayer`]: super::ReconcileLayer
pub trait Reconciler<M: Modality>: Send + Sync {
    /// Decide the disposition of the grouped pair `a`, `b`.
    fn decide(&self, a: &Entity<M>, b: &Entity<M>) -> Disposition;

    /// Stable name, recorded on the provenance of merged / resolved /
    /// contested entities.
    fn name(&self) -> &'static str;
}
