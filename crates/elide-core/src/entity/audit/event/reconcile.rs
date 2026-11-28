//! Reconciliation payloads: what happened to an entity as detections were
//! combined and scored, fusion ([`Fused`]), cross-label arbitration
//! ([`Resolved`] / [`Contested`]), confidence rescaling ([`Calibrated`]),
//! context-keyword boosting ([`Refined`]), and leaving a finding out
//! ([`Dropped`]).
//!
//! Each payload declares a central `TAG` discriminant byte, written before its
//! own bytes so two kinds can never hash alike, see the [payloads
//! overview](super).

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::super::hash::AuditHasher;
use crate::entity::LabelRef;
use crate::modality::{Hint, Modality, ModalityLocation};
use crate::primitive::{ComponentId, Confidence};

/// Several detections of the same finding were fused into this one entity,
/// located at the span that covers them all.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        bound = "M: schemars::JsonSchema, M::Location: schemars::JsonSchema",
        rename = "{M}Fused"
    )
)]
pub struct Fused<M: Modality> {
    /// The scoring that pooled their confidences.
    pub strategy: ComponentId,
    /// Where the fused entity sits: the union of the detections' locations.
    pub location: M::Location,
}

impl<M: Modality> Fused<M> {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 2;

    /// The `strategy` fused detections into one at `location`, the union of
    /// theirs.
    pub fn new(strategy: ComponentId, location: M::Location) -> Self {
        Self { strategy, location }
    }

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.component(&self.strategy);
        out.bytes(&self.location.hash());
    }
}

/// A competing detection of a *different* label lost to this entity.
///
/// It covered the same span and was resolved against this (winning) entity.
/// The loser is kept beside it, its own trail holding a [`Dropped`] event
/// superseded by this entity, unless the analyzer discards what it drops.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Resolved {
    /// The id of the entity that lost arbitration. An analyzer that discards
    /// what it drops leaves this naming no entity.
    pub competing_id: Uuid,
    /// The label of the detection that lost arbitration.
    pub competing_label: LabelRef,
    /// The loser's confidence at resolution time.
    pub competing_confidence: Confidence,
    /// The reconciler that chose the winner.
    pub resolved_by: ComponentId,
}

impl Resolved {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 3;

    /// The loser of a cross-label arbitration, `competing_id` with its
    /// `competing_label` and `competing_confidence`, and the reconciler
    /// (`resolved_by`) that chose the winner.
    pub fn new(
        competing_id: Uuid,
        competing_label: LabelRef,
        competing_confidence: Confidence,
        resolved_by: ComponentId,
    ) -> Self {
        Self {
            competing_id,
            competing_label,
            competing_confidence,
            resolved_by,
        }
    }

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.raw(self.competing_id.as_bytes());
        out.bytes(self.competing_label.as_str().as_bytes());
        out.raw(&self.competing_confidence.get().to_le_bytes());
        out.component(&self.resolved_by);
    }
}

/// A competing detection of a different label was left *unresolved*.
///
/// It covered the same span; both entities survive, flagged for a human.
/// Recorded on each entity of the contested pair, naming the other.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Contested {
    /// The id of the competing entity. A fusion that absorbs it afterwards
    /// leaves this naming no entity.
    pub competing_id: Uuid,
    /// The label of the competing detection.
    pub competing_label: LabelRef,
    /// The competing detection's confidence.
    pub competing_confidence: Confidence,
    /// The reconciler that flagged the contest.
    pub flagged_by: ComponentId,
}

impl Contested {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 4;

    /// The other side of an unresolved cross-label overlap, `competing_id`
    /// with its `competing_label` and `competing_confidence`, and the
    /// reconciler (`flagged_by`) that flagged the contest.
    pub fn new(
        competing_id: Uuid,
        competing_label: LabelRef,
        competing_confidence: Confidence,
        flagged_by: ComponentId,
    ) -> Self {
        Self {
            competing_id,
            competing_label,
            competing_confidence,
            flagged_by,
        }
    }

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.raw(self.competing_id.as_bytes());
        out.bytes(self.competing_label.as_str().as_bytes());
        out.raw(&self.competing_confidence.get().to_le_bytes());
        out.component(&self.flagged_by);
    }
}

/// Detection left this entity out of its findings: a layer rejected it, or
/// another entity superseded it.
///
/// The entity is kept, so the trail says why it is missing, but redaction
/// leaves it alone unless a reviewer confirms it, see
/// [`AuditLog::is_suppressed`].
///
/// [`AuditLog::is_suppressed`]: crate::entity::audit::AuditLog::is_suppressed
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Dropped {
    /// The layer that dropped it.
    pub by: ComponentId,
    /// The entity that won over it, when it lost an overlap rather than being
    /// rejected outright.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub superseded_by: Option<Uuid>,
}

impl Dropped {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 13;

    /// A rejection by `by`: the entity was left out on its own account.
    pub fn rejected(by: ComponentId) -> Self {
        Self {
            by,
            superseded_by: None,
        }
    }

    /// A supersession by `by`: the entity `winner` was kept over it.
    pub fn superseded(by: ComponentId, winner: Uuid) -> Self {
        Self {
            by,
            superseded_by: Some(winner),
        }
    }

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.component(&self.by);
        match self.superseded_by {
            Some(winner) => {
                out.byte(1);
                out.raw(winner.as_bytes());
            }
            None => {
                out.byte(0);
            }
        }
    }
}

/// The entity's confidence was rescaled by the factor its recognizer's
/// calibration gives its label.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Calibrated {
    /// The recognizer whose scores were rescaled.
    pub recognizer: ComponentId,
    /// Multiplier applied.
    pub factor: f64,
}

impl Calibrated {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 5;

    /// The `recognizer`'s score rescaled by `factor`.
    pub fn new(recognizer: ComponentId, factor: f64) -> Self {
        Self { recognizer, factor }
    }

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.component(&self.recognizer);
        out.raw(&self.factor.to_bits().to_le_bytes());
    }
}

/// A context keyword near the entity lifted its confidence.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        bound = "M: schemars::JsonSchema, M::Location: schemars::JsonSchema",
        rename = "{M}Refined"
    )
)]
pub struct Refined<M: Modality> {
    /// The recognizer whose context keyword fired.
    pub recognizer: ComponentId,
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

impl<M: Modality> Refined<M> {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 6;

    /// A refinement fired by the `recognizer`'s `keyword`, with no located
    /// hint or resolved location. Attach them with
    /// [`with_hint`](Self::with_hint) / [`with_location`](Self::with_location).
    pub fn new(recognizer: ComponentId, keyword: impl Into<HipStr<'static>>) -> Self {
        Self {
            recognizer,
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
        out.component(&self.recognizer);
        out.bytes(self.keyword.as_bytes());
        // The hint's location identifies where the keyword sits; its data
        // payload is auxiliary context and stays out of the hash.
        out.opt(self.hint.as_ref().map(|h| h.location.hash()).as_deref());
        out.opt(self.location.as_ref().map(|l| l.hash()).as_deref());
    }
}
