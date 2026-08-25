//! Application payloads: the redaction decision and its execution
//! ([`Selection`] then [`Redaction`]), and human overrides ([`Manual`]).
//!
//! Each payload declares a central `TAG` discriminant byte, written before its
//! own bytes so two kinds can never hash alike — see the [payloads
//! overview](super).

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::super::hash::AuditHasher;
use super::super::{Attribution, AuditHash, RuleMatch};
use crate::modality::{Modality, ModalityLocation};
use crate::operator::{LeakProfile, OperatorId};

/// An operator hid the entity.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Redaction {
    /// Which operator (name + version) ran.
    pub operator: OperatorId,
    /// How much the output leaks about the original, when the operator claimed a
    /// profile; `None` when it made no claim.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub leak_profile: Option<LeakProfile>,
    /// Identifier of the key needed to reverse it, if reversible.
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub key_id: Option<HipStr<'static>>,
    /// Which selection rule chose this operator: the automatic "why" (matched a
    /// label, a tag, a predicate, or the fallback).
    pub matched_by: RuleMatch,
    /// The author-supplied policy rationale, when the operator carried an
    /// [`Attribution`]; `None` otherwise.
    pub attribution: Option<Attribution>,
    /// BLAKE3 digest of the original text the operator hid, when the redaction
    /// layer recorded it. Proves *what* was redacted without storing the
    /// plaintext; `None` when the operator did not capture it.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub span_hash: Option<AuditHash>,
    /// Byte length of the original text the operator hid, paired with
    /// [`span_hash`](Self::span_hash). `None` when not captured.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub span_length: Option<u32>,
}

impl Redaction {
    /// This kind's discriminant byte (see the [payloads overview](super)).
    pub(crate) const TAG: u8 = 7;

    /// A redaction by `operator`, chosen by the rule `matched_by`. The leak
    /// profile, reversal key, policy attribution, and captured span are attached
    /// with the `with_*` builders.
    pub fn new(operator: OperatorId, matched_by: RuleMatch) -> Self {
        Self {
            operator,
            leak_profile: None,
            key_id: None,
            matched_by,
            attribution: None,
            span_hash: None,
            span_length: None,
        }
    }

    /// Attach how much the operator's output leaks about the original.
    #[must_use]
    pub fn with_leak_profile(mut self, leak_profile: LeakProfile) -> Self {
        self.leak_profile = Some(leak_profile);
        self
    }

    /// Attach the identifier of the key needed to reverse a reversible operator.
    #[must_use]
    pub fn with_key_id(mut self, key_id: impl Into<HipStr<'static>>) -> Self {
        self.key_id = Some(key_id.into());
        self
    }

    /// Attach the author-supplied policy [`Attribution`] the operator carried.
    #[must_use]
    pub fn with_attribution(mut self, attribution: impl Into<Attribution>) -> Self {
        self.attribution = Some(attribution.into());
        self
    }

    /// Record what was hidden without the plaintext: the BLAKE3 `hash` of the
    /// original span and its byte `length`. The two are set together — they are
    /// meaningless apart.
    #[must_use]
    pub fn with_span(mut self, hash: AuditHash, length: u32) -> Self {
        self.span_hash = Some(hash);
        self.span_length = Some(length);
        self
    }

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.bytes(self.operator.name.as_bytes());
        out.bytes(self.operator.version.as_bytes());
        match self.leak_profile {
            Some(profile) => {
                out.byte(1);
                out.byte(profile as u8);
            }
            None => {
                out.byte(0);
            }
        }
        out.opt(self.key_id.as_ref().map(|s| s.as_bytes()));
        self.matched_by.hash_into(out);
        hash_opt_attribution(out, self.attribution.as_ref());
        out.opt(self.span_hash.as_ref().map(|h| h.as_bytes().as_slice()));
        match self.span_length {
            Some(length) => {
                out.byte(1);
                out.raw(&length.to_le_bytes());
            }
            None => {
                out.byte(0);
            }
        }
    }
}

/// An operator was *picked* to hide the entity — the redaction decision,
/// recorded before it is applied so it can be reviewed (and the entity edited)
/// first. The [`Redaction`] event that follows records the operator actually
/// run.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Selection {
    /// Identity (name + version) of the operator picked. Its *config* is not
    /// recorded — it lives in the policy that will run it, so apply re-resolves
    /// the configured operator rather than reading it here.
    pub operator: OperatorId,
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

    /// A pick of `operator`, chosen by the rule `matched_by`, with no policy
    /// attribution. Attach one with [`with_attribution`](Self::with_attribution).
    pub fn new(operator: OperatorId, matched_by: RuleMatch) -> Self {
        Self {
            operator,
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
        self.matched_by.hash_into(out);
        hash_opt_attribution(out, self.attribution.as_ref());
    }
}

/// A human override, outside automatic detection: an entity a reviewer added by
/// hand, or a detected one they marked to ignore. Its provenance is a person's
/// decision, not a recognizer's — so the trail records *why* (an
/// [`Attribution`], when supplied). *Who* made the override is the event's
/// [`source`], not a payload field.
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
    /// is redacted — [`AuditLog::is_suppressed`] reads it, so there is no
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

/// Which human decision a [`Manual`] event records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ManualIntent {
    /// A reviewer added an entity detection missed. The entity is redacted like
    /// any detected one.
    Include,
    /// A reviewer marked a detected entity to leave alone (a false positive).
    /// The redaction pass skips it — see [`AuditLog::is_suppressed`].
    ///
    /// [`AuditLog::is_suppressed`]: crate::entity::audit::AuditLog::is_suppressed
    Suppress,
}

/// Fold an optional [`Attribution`] into `out`: a presence byte, then the
/// attribution's own bytes if present. Shared by [`Redaction`] and [`Selection`].
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
