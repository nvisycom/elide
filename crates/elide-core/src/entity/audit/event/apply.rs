//! Application payloads: the redaction decision and its execution
//! ([`Selected`] then [`Redacted`]), and a reviewer's work: an entity they
//! added ([`Included`]), and their decisions about one ([`Manual`]).
//!
//! Each payload declares a central `TAG` discriminant byte, written before its
//! own bytes so two kinds can never hash alike, see the [payloads
//! overview](super).

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::super::hash::{AuditHash, AuditHasher};
use super::super::{Attribution, RuleMatch};
use crate::entity::LabelRef;
use crate::modality::{Modality, ModalityLocation};
use crate::primitive::ComponentId;
use crate::redaction::LeakProfile;

/// The redaction *decision*: which operator was picked to hide the entity.
///
/// It is recorded before the operator is applied, so it can be reviewed (and
/// the entity edited) first.
///
/// It carries the whole decision: which operator, how much its output leaks,
/// which rule chose it, and the policy's rationale. The [`Redacted`] event that
/// applies it names this event by its hash.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Selected {
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

impl Selected {
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

/// The redaction *execution*: the entity was hidden by applying the decision
/// a [`Selected`] event records, at a location.
///
/// It names that selection event by its hash, so which operator ran, why, and
/// how much its output leaks are read from the decision itself, see
/// [`AuditLog::applied_selection`]. [`AuditLog::verify`] checks the link names
/// an earlier selection event on the same trail. `location` is where the
/// operator wrote: the entity's location, with a mark resolved to the span it
/// covers.
///
/// [`AuditLog::applied_selection`]: crate::entity::audit::AuditLog::applied_selection
/// [`AuditLog::verify`]: crate::entity::audit::AuditLog::verify
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
        rename = "{M}Redacted"
    )
)]
pub struct Redacted<M: Modality> {
    /// The hash of the [`Selected`] event this redaction applied.
    pub selection: AuditHash,
    /// Where the operator wrote.
    pub location: M::Location,
}

impl<M: Modality> Redacted<M> {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 7;

    /// A redaction applying the [`Selected`] event whose hash is `selection`
    /// at `location`.
    pub fn new(selection: AuditHash, location: M::Location) -> Self {
        Self {
            selection,
            location,
        }
    }

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.raw(self.selection.as_bytes());
        out.bytes(&self.location.hash());
    }
}

/// A reviewer's inclusion: an entity of `label` at `location` that detection
/// missed (a false negative). It starts the entity's trail, never follows a
/// birth, and the entity is redacted like any detected one.
///
/// Who included it is the event's [`source`], not a payload field.
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
        rename = "{M}Included"
    )
)]
pub struct Included<M: Modality> {
    /// The label the reviewer gave the entity.
    pub label: LabelRef,
    /// Where it sits.
    pub location: M::Location,
    /// Why it was included, when the reviewer said.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub attribution: Option<Attribution>,
}

impl<M: Modality> Included<M> {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 12;

    /// An inclusion of `label` at `location`, with no reason given.
    pub fn new(label: impl Into<LabelRef>, location: M::Location) -> Self {
        Self {
            label: label.into(),
            location,
            attribution: None,
        }
    }

    /// Give the reason for the inclusion.
    #[must_use]
    pub fn because(mut self, attribution: impl Into<Attribution>) -> Self {
        self.attribution = Some(attribution.into());
        self
    }

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.bytes(self.label.as_str().as_bytes());
        out.bytes(&self.location.hash());
        hash_opt_attribution(out, self.attribution.as_ref());
    }
}

/// A reviewer's decision about an entity: they suppressed, confirmed,
/// relabelled, or relocated it.
///
/// Its provenance is a person's decision, not a recognizer's, so the trail
/// records what the decision set and *why* (an [`Attribution`], when
/// supplied). *Who* made the override is the event's [`source`], not a payload
/// field.
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
    /// Which decision this records, with the label it set when it set one. This
    /// is the authority on whether the entity is redacted,
    /// [`AuditLog::is_suppressed`] reads it, so there is no separate flag to
    /// keep in sync.
    ///
    /// [`AuditLog::is_suppressed`]: crate::entity::audit::AuditLog::is_suppressed
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub intent: ManualIntent,
    /// Where the decision applies, in modality-native coordinates: the
    /// entity's location after a relocation, or the region a verdict covered.
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
        self.intent.hash_into(out);
        out.bytes(&self.location.hash());
        hash_opt_attribution(out, self.attribution.as_ref());
    }
}

/// Which reviewer decision a [`Manual`] event records, with what it set.
///
/// A verdict, [`Suppress`](ManualIntent::Suppress) a false positive or
/// [`Confirm`](ManualIntent::Confirm) a finding, or an amendment,
/// [`Relabel`](ManualIntent::Relabel) it or
/// [`Relocate`](ManualIntent::Relocate) it to the event's location. A miss a
/// reviewer adds is an [`Included`] event, which starts its entity.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "intent", rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(tag = "intent", rename_all = "snake_case")
)]
pub enum ManualIntent {
    /// A reviewer marked a detected entity to leave alone (a false positive).
    /// The redaction pass skips it, see [`AuditLog::is_suppressed`].
    ///
    /// [`AuditLog::is_suppressed`]: crate::entity::audit::AuditLog::is_suppressed
    Suppress,
    /// A reviewer gave the entity the label `label`.
    Relabel {
        /// The label the entity has from this event on.
        label: LabelRef,
    },
    /// A reviewer moved the entity to the event's location.
    Relocate,
    /// A reviewer confirmed a finding (a true positive): it is redacted, even
    /// after an earlier [`Suppress`](Self::Suppress).
    Confirm,
}

impl ManualIntent {
    /// This intent's discriminant byte, folded into the hash. Pinned so a
    /// reordered or inserted variant cannot silently change a recorded hash;
    /// append new variants with the next value. 0 is retired and never reused.
    const fn tag(&self) -> u8 {
        match self {
            Self::Suppress => 1,
            Self::Relabel { .. } => 2,
            Self::Relocate => 3,
            Self::Confirm => 4,
        }
    }

    fn hash_into(&self, out: &mut AuditHasher) {
        out.byte(self.tag());
        match self {
            Self::Relabel { label } => {
                out.bytes(label.as_str().as_bytes());
            }
            Self::Suppress | Self::Relocate | Self::Confirm => {}
        }
    }
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
