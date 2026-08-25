//! Reconciliation payloads: what happened to an entity as detections were
//! combined and scored — fusion ([`Deduplication`]), cross-label arbitration
//! ([`Conflict`] / [`Contested`]), confidence rescaling ([`Calibration`]), and
//! context-keyword boosting ([`Refinement`]).
//!
//! Each payload declares a central `TAG` discriminant byte, written before its
//! own bytes so two kinds can never hash alike — see the [payloads
//! overview](super).

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::super::hash::AuditHasher;
use crate::entity::LabelRef;
use crate::modality::{Hint, Modality, ModalityLocation};
use crate::primitive::Confidence;

/// Several detections were fused into one entity.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Deduplication {
    /// Name of the fusion strategy that combined them.
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub strategy: HipStr<'static>,
}

impl Deduplication {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 2;

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.bytes(self.strategy.as_bytes());
    }
}

/// A competing detection of a *different* label over the same span was resolved
/// against this (winning) entity — the loser is recorded, not dropped.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Conflict {
    /// The label of the detection that lost arbitration.
    pub competing_label: LabelRef,
    /// The loser's confidence at resolution time.
    pub competing_confidence: Confidence,
    /// Name of the conflict policy that chose the winner.
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub resolved_by: HipStr<'static>,
}

impl Conflict {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 3;

    /// The loser of a cross-label arbitration: its `competing_label` and
    /// `competing_confidence`, and the policy (`resolved_by`) that chose the
    /// winner.
    pub fn new(
        competing_label: LabelRef,
        competing_confidence: Confidence,
        resolved_by: impl Into<HipStr<'static>>,
    ) -> Self {
        Self {
            competing_label,
            competing_confidence,
            resolved_by: resolved_by.into(),
        }
    }

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.bytes(self.competing_label.as_str().as_bytes());
        out.raw(&self.competing_confidence.get().to_le_bytes());
        out.bytes(self.resolved_by.as_bytes());
    }
}

/// A competing detection of a different label over the same span was left
/// *unresolved*: both entities survive, flagged for a human. Recorded on each
/// entity of the contested pair, naming the other.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Contested {
    /// The label of the competing detection.
    pub competing_label: LabelRef,
    /// The competing detection's confidence.
    pub competing_confidence: Confidence,
    /// Name of the policy that flagged the contest.
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub flagged_by: HipStr<'static>,
}

impl Contested {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 4;

    /// The other side of an unresolved cross-label overlap: its
    /// `competing_label` and `competing_confidence`, and the policy
    /// (`flagged_by`) that flagged the contest.
    pub fn new(
        competing_label: LabelRef,
        competing_confidence: Confidence,
        flagged_by: impl Into<HipStr<'static>>,
    ) -> Self {
        Self {
            competing_label,
            competing_confidence,
            flagged_by: flagged_by.into(),
        }
    }

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.bytes(self.competing_label.as_str().as_bytes());
        out.raw(&self.competing_confidence.get().to_le_bytes());
        out.bytes(self.flagged_by.as_bytes());
    }
}

/// The entity's confidence was rescaled by a per-recognizer factor.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Calibration {
    /// Multiplier applied.
    pub factor: f64,
}

impl Calibration {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 5;

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.raw(&self.factor.to_bits().to_le_bytes());
    }
}

/// A context keyword near the entity lifted its confidence.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>, \
                   M::Data: Serialize + for<'a> Deserialize<'a>")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        bound = "M: schemars::JsonSchema, M::Location: schemars::JsonSchema, M::Data: schemars::JsonSchema",
        rename = "{M}Refinement"
    )
)]
pub struct Refinement<M: Modality> {
    /// Keyword that fired the boost.
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub keyword: HipStr<'static>,
    /// The located [`Hint`] the keyword fired from, when the match came from an
    /// out-of-band hint (a column header, a key) rather than the in-text word
    /// window. `None` for an in-text-window match.
    ///
    /// [`Hint`]: crate::modality::Hint
    pub hint: Option<Hint<M>>,
    /// Where the boosting keyword sits in the medium. For a hint match this
    /// mirrors the hint's own location; for an in-text-window match it is the
    /// keyword resolved through the modality's [`locate`]. `None` when the
    /// keyword's stream range could not be placed.
    ///
    /// [`locate`]: crate::modality::TextRecognizable::locate
    pub location: Option<M::Location>,
}

impl<M: Modality> Refinement<M> {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 6;

    /// A refinement fired by `keyword`, with no located hint or resolved
    /// location. Attach them with [`with_hint`](Self::with_hint) /
    /// [`with_location`](Self::with_location).
    pub fn new(keyword: impl Into<HipStr<'static>>) -> Self {
        Self {
            keyword: keyword.into(),
            hint: None,
            location: None,
        }
    }

    /// Attach the located [`Hint`] the keyword fired from (an out-of-band match).
    ///
    /// [`Hint`]: crate::modality::Hint
    #[must_use]
    pub fn with_hint(mut self, hint: Hint<M>) -> Self {
        self.hint = Some(hint);
        self
    }

    /// Attach where the boosting keyword sits in the medium.
    #[must_use]
    pub fn with_location(mut self, location: M::Location) -> Self {
        self.location = Some(location);
        self
    }

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.bytes(self.keyword.as_bytes());
        // The hint's location identifies where the keyword sits; its data
        // payload is auxiliary context and stays out of the hash.
        out.opt(self.hint.as_ref().map(|h| h.location.hash()).as_deref());
        out.opt(self.location.as_ref().map(|l| l.hash()).as_deref());
    }
}
