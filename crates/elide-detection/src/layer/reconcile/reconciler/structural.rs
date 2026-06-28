//! The [`Structural`] reconciler and its [`OnConflict`] knob.

use elide_core::entity::Entity;
use elide_core::modality::{Modality, ModalityLocation, Overlap};

use super::super::tiebreaker::{HighestConfidence, Tiebreaker};
use super::{Disposition, Reconciler, Winner};

/// How a [`Structural`] reconciler disposes of a *true* conflict — two
/// confident, near-coincident, differently-labelled findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnConflict {
    /// Pick a winner with the tiebreaker; drop and record the loser. Clean,
    /// machine-decided output.
    #[default]
    Resolve,
    /// Keep both, flag them contested, and defer to the human edit step.
    Contest,
}

/// The default, structure-aware reconciler.
///
/// Reads the geometric relationship between the two locations rather than
/// treating every overlap as a contest:
/// - **Nesting** — one location inside the other (a postal code within an
///   address): a legitimate hierarchy → keep both, *unless* the contained
///   entity is much weaker than its container (`inner + margin < outer`), a
///   subsumed junk match that loses.
/// - **Incidental overlap** — they overlap but their [intersection-over-union]
///   is below `threshold`: two distinct findings that merely touch → keep both.
/// - **Near-coincident** — IoU at or above `threshold`: two recognizers
///   claiming substantially the same span with different labels, a true
///   conflict → disposed of per [`on_conflict`] (resolve with `tiebreaker`, or
///   contest for human review).
///
/// [intersection-over-union]: elide_core::modality::Overlap::Crossing
/// [`on_conflict`]: Structural::on_conflict
#[derive(Debug, Clone, Copy)]
pub struct Structural<T = HighestConfidence> {
    /// The IoU at or above which a non-nested overlap counts as a conflict
    /// rather than two distinct findings.
    pub threshold: f32,
    /// The confidence margin within which a *contained* entity is kept as a
    /// real nesting; a contained entity weaker than `container − margin` is
    /// subsumed and dropped.
    pub margin: f32,
    /// How a true conflict is settled.
    pub tiebreaker: T,
    /// Whether a true conflict is auto-resolved or surfaced for review.
    pub on_conflict: OnConflict,
}

impl<T> Structural<T> {
    /// A structural reconciler with the given parameters.
    pub fn new(threshold: f32, margin: f32, tiebreaker: T, on_conflict: OnConflict) -> Self {
        Self {
            threshold,
            margin,
            tiebreaker,
            on_conflict,
        }
    }
}

impl Structural<HighestConfidence> {
    /// The default: IoU threshold `0.5`, nesting margin `0.25`, ties to the
    /// higher confidence, true conflicts auto-resolved.
    pub fn standard() -> Self {
        Self {
            threshold: 0.5,
            margin: 0.25,
            tiebreaker: HighestConfidence,
            on_conflict: OnConflict::Resolve,
        }
    }

    /// The standard parameters, but true conflicts are *contested* (surfaced
    /// for the human edit step) instead of auto-resolved.
    pub fn reviewing() -> Self {
        Self {
            on_conflict: OnConflict::Contest,
            ..Self::standard()
        }
    }
}

impl Default for Structural<HighestConfidence> {
    fn default() -> Self {
        Self::standard()
    }
}

impl<M, T> Reconciler<M> for Structural<T>
where
    M: Modality,
    T: Tiebreaker<M>,
{
    fn decide(&self, a: &Entity<M>, b: &Entity<M>) -> Disposition {
        // A contained span weaker than its container by more than `margin` is
        // subsumed noise; the container wins. An otherwise-confident nesting is
        // a legitimate hierarchy — keep both.
        let subsumed = |inner: &Entity<M>, outer: &Entity<M>| {
            inner.confidence.get() + self.margin < outer.confidence.get()
        };
        match a.location.overlap(&b.location) {
            // `b` is inside `a`.
            Overlap::Contains if subsumed(b, a) => Disposition::Resolve {
                winner: Winner::First,
            },
            // `a` is inside `b`.
            Overlap::ContainedBy if subsumed(a, b) => Disposition::Resolve {
                winner: Winner::Second,
            },
            // A legitimate nesting, or two distinct findings that merely touch
            // (IoU below the threshold): keep both.
            Overlap::Disjoint | Overlap::Contains | Overlap::ContainedBy => Disposition::KeepBoth,
            Overlap::Crossing { iou } if iou < self.threshold => Disposition::KeepBoth,
            // Substantially the same span: a true conflict.
            Overlap::Crossing { .. } => self.settle(a, b),
        }
    }

    fn name(&self) -> &'static str {
        "structural"
    }
}

impl<T> Structural<T> {
    /// Settle a true conflict per [`on_conflict`]: resolve with the tiebreaker,
    /// or contest for review.
    ///
    /// [`on_conflict`]: Self::on_conflict
    fn settle<M>(&self, a: &Entity<M>, b: &Entity<M>) -> Disposition
    where
        M: Modality,
        T: Tiebreaker<M>,
    {
        match self.on_conflict {
            OnConflict::Contest => Disposition::Contest,
            OnConflict::Resolve => Disposition::Resolve {
                winner: if self.tiebreaker.keeps_first(a, b) {
                    Winner::First
                } else {
                    Winner::Second
                },
            },
        }
    }
}
