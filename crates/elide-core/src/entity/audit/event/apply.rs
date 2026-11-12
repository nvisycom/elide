//! Application payloads: the redaction decision and its execution
//! ([`Selection`] then [`Redaction`]), and human overrides ([`Manual`]).
//!
//! Each payload declares a central `TAG` discriminant byte, written before its
//! own bytes so two kinds can never hash alike, see the [payloads
//! overview](super).

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::super::hash::{AuditHash, AuditHasher};
use super::super::{Attribution, RuleMatch};
use crate::modality::{Modality, ModalityLocation};
use crate::primitive::ComponentId;
use crate::redaction::LeakProfile;

/// The redaction *decision*: which operator was picked to hide the entity.
///
/// It is recorded before the operator is applied, so it can be reviewed (and
/// the entity edited) first.
///
/// It carries the whole decision: which operator, how much its output leaks,
/// which rule chose it, and the policy's rationale. The [`Redaction`] that
/// applies it names this event by its hash.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Selection {
    /// Identity (name + version) of the operator picked. Its *config* is not
    /// recorded; it lives in the policy that will run it, so apply re-resolves
    /// the configured operator rather than reading it here.
    pub operator: ComponentId,
    /// How much the operator's output leaks about the original.
    pub leak_profile: LeakProfile,
    /// Which selection rule chose this operator: the automatic "why" (matched a
    /// label, a tag, a predicate, or the fallback).
    pub matched_by: RuleMatch,
    /// The author-supplied policy rationale, when the matched rule carried an
    /// [`Attribution`]; `None` otherwise.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub attribution: Option<Attribution>,
}

impl Selection {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 9;

    /// A pick of `operator`, whose output leaks as `leak_profile`, chosen by the
    /// rule `matched_by`, with no policy attribution. Attach one with
    /// [`with_attribution`](Self::with_attribution).
    pub fn new(operator: ComponentId, leak_profile: LeakProfile, matched_by: RuleMatch) -> Self {
        Self {
            operator,
            leak_profile,
            matched_by,
            attribution: None,
        }
    }

    /// Attach the author-supplied policy [`Attribution`] the matched rule carried.
    #[must_use]
    pub fn with_attribution(mut self, attribution: impl Into<Attribution>) -> Self {
        self.attribution = Some(attribution.into());
        self
    }

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.bytes(self.operator.name.as_bytes());
        out.bytes(self.operator.version.as_bytes());
        out.byte(self.leak_profile as u8);
        self.matched_by.hash_into(out);
        hash_opt_attribution(out, self.attribution.as_ref());
    }
}

/// The redaction *execution*: the entity was hidden by applying a
/// [`Selection`].
///
/// It names that selection event by its hash, so which operator ran, why, and
/// how much its output leaks are read from the decision itself, see
/// [`AuditLog::applied_selection`]. [`AuditLog::verify`] checks the link names
/// an earlier selection event on the same trail.
///
/// [`AuditLog::applied_selection`]: crate::entity::audit::AuditLog::applied_selection
/// [`AuditLog::verify`]: crate::entity::audit::AuditLog::verify
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Redaction {
    /// The hash of the [`Selection`] event this redaction applied.
    pub selection: AuditHash,
}

impl Redaction {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 7;

    /// A redaction applying the [`Selection`] event whose hash is `selection`.
    pub fn new(selection: AuditHash) -> Self {
        Self { selection }
    }

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.raw(self.selection.as_bytes());
    }
}

/// A human override: an entity a reviewer added, flagged, or suppressed.
///
/// An entity a reviewer added by hand, or a detected one they marked to ignore.
/// Its provenance is a person's decision, not a recognizer's, so the trail
/// records *why* (an [`Attribution`], when supplied). *Who* made the override
/// is the event's [`source`], not a payload field.
///
/// [`source`]: super::AuditEvent::source
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
        rename = "{M}Manual"
    )
)]
pub struct Manual<M: Modality> {
    /// Which human decision this records: including a missed entity, or
    /// suppressing a detected one. This is the authority on whether the entity
    /// is redacted, [`AuditLog::is_suppressed`] reads it, so there is no
    /// separate flag to keep in sync.
    ///
    /// [`AuditLog::is_suppressed`]: crate::entity::audit::AuditLog::is_suppressed
    pub intent: ManualIntent,
    /// Where the override applies, in modality-native coordinates.
    pub location: M::Location,
    /// The reviewer's rationale, when supplied (e.g. a freeform
    /// `"false positive"`, or a cited authority). `None` for an unexplained
    /// override.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub attribution: Option<Attribution>,
}

impl<M: Modality> Manual<M> {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 8;

    /// A human override recording `intent` at `location`, with no rationale.
    /// Attach one with [`with_attribution`](Self::with_attribution). *Who* made
    /// the override is the event's source, not set here.
    pub fn new(intent: ManualIntent, location: M::Location) -> Self {
        Self {
            intent,
            location,
            attribution: None,
        }
    }

    /// Attach the reviewer's rationale [`Attribution`] for the override.
    #[must_use]
    pub fn with_attribution(mut self, attribution: impl Into<Attribution>) -> Self {
        self.attribution = Some(attribution.into());
        self
    }

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.byte(self.intent as u8);
        out.bytes(&self.location.hash());
        hash_opt_attribution(out, self.attribution.as_ref());
    }
}

/// Which reviewer decision a [`Manual`] event records about a finding.
///
/// [`Flag`](ManualIntent::Flag) a miss, [`Suppress`](ManualIntent::Suppress) a
/// false positive, or [`Amend`](ManualIntent::Amend) an existing finding.
// Explicit discriminants: `Manual::hash_into` folds `intent as u8` into the
// tamper-evident hash, so these values are part of the hash. Pinning them makes
// that dependence visible and keeps a reorder or mid-enum insertion from
// silently changing an event's hash. Append new variants with the next value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ManualIntent {
    /// A reviewer flagged an entity detection missed (a false negative). The
    /// entity is redacted like any detected one.
    Flag = 0,
    /// A reviewer marked a detected entity to leave alone (a false positive).
    /// The redaction pass skips it, see [`AuditLog::is_suppressed`].
    ///
    /// [`AuditLog::is_suppressed`]: crate::entity::audit::AuditLog::is_suppressed
    Suppress = 1,
    /// A reviewer amended an existing entity, retagged its label, adjusted its
    /// span, or changed another attribute. The entity stays and is redacted
    /// normally; this records that a human changed it.
    Amend = 2,
}

/// Fold an optional [`Attribution`] into `out`: a presence byte, then the
/// attribution's own bytes if present. Shared by every payload that carries one.
fn hash_opt_attribution(out: &mut AuditHasher, attribution: Option<&Attribution>) {
    match attribution {
        Some(attribution) => {
            out.byte(1);
            attribution.hash_into(out);
        }
        None => {
            out.byte(0);
        }
    }
}
