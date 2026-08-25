//! The payload of each [`AuditKind`] variant: one struct per kind, owning its
//! fields, its documentation, and how it folds into the tamper-evident hash.
//!
//! [`AuditKind`] is a thin tagged union over these payloads. Keeping a kind's
//! data next to its `hash` — instead of smearing the two across a variant list
//! and a monolithic match — is what makes a kind readable as a unit: the fields
//! and exactly how they are hashed sit together.
//!
//! Each payload declares a central [`TAG`] discriminant byte, written before its
//! own bytes so two kinds can never hash alike. The audit chain's
//! tamper-evidence depends on those bytes being unique: every `TAG` is an
//! inherent `const` declared once here, so a collision is a duplicated `const`
//! visible at a glance, rather than a stray `push`. Kinds are core-owned, so the
//! discriminant space stays centrally assigned.
//!
//! [`AuditKind`]: super::AuditKind
//! [`TAG`]: Pattern::TAG

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::event::{put_bytes, put_opt};
use super::{Attribution, AuditHash, RuleMatch};
use crate::entity::LabelRef;
use crate::modality::{Hint, Modality, ModalityLocation};
use crate::operator::{LeakProfile, OperatorId};
use crate::primitive::Confidence;

/// Detail of a pattern/dictionary recognition: a recognizer matched at
/// `location`, with the pattern metadata in `pattern`.
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
        rename = "{M}Pattern"
    )
)]
pub struct Pattern<M: Modality> {
    /// Where the recognizer matched.
    pub location: M::Location,
    /// Pattern metadata (name, regex, validator, contextual flag).
    pub pattern: PatternEvent,
}

impl<M: Modality> Pattern<M> {
    /// This kind's discriminant byte. See the [module docs](self) — unique
    /// across all kinds, written before the payload's own bytes.
    pub(crate) const TAG: u8 = 0;

    pub(crate) fn hash(&self, out: &mut Vec<u8>) {
        put_bytes(out, &self.location.hash());
        put_bytes(out, self.pattern.name.as_bytes());
        put_opt(out, self.pattern.regex.as_ref().map(|s| s.as_bytes()));
        put_opt(out, self.pattern.validator.as_ref().map(|s| s.as_bytes()));
        out.push(self.pattern.contextual.into());
    }
}

/// Detail of a model/NER recognition: a model matched at `location`, with its
/// metadata in `model`.
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
        rename = "{M}Model"
    )
)]
pub struct Model<M: Modality> {
    /// Where the recognizer matched.
    pub location: M::Location,
    /// Model metadata (name, version, contextual flag).
    pub model: ModelEvent,
}

impl<M: Modality> Model<M> {
    /// This kind's discriminant byte (see the [module docs](self)).
    pub(crate) const TAG: u8 = 1;

    pub(crate) fn hash(&self, out: &mut Vec<u8>) {
        put_bytes(out, &self.location.hash());
        put_bytes(out, self.model.name.as_bytes());
        put_opt(out, self.model.version.as_ref().map(|s| s.as_bytes()));
        out.push(self.model.contextual.into());
    }
}

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
    /// This kind's discriminant byte (see the [module docs](self)).
    pub(crate) const TAG: u8 = 2;

    pub(crate) fn hash(&self, out: &mut Vec<u8>) {
        put_bytes(out, self.strategy.as_bytes());
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
    /// This kind's discriminant byte (see the [module docs](self)).
    pub(crate) const TAG: u8 = 3;

    pub(crate) fn hash(&self, out: &mut Vec<u8>) {
        put_bytes(out, self.competing_label.as_str().as_bytes());
        out.extend_from_slice(&self.competing_confidence.get().to_le_bytes());
        put_bytes(out, self.resolved_by.as_bytes());
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
    /// This kind's discriminant byte (see the [module docs](self)).
    pub(crate) const TAG: u8 = 4;

    pub(crate) fn hash(&self, out: &mut Vec<u8>) {
        put_bytes(out, self.competing_label.as_str().as_bytes());
        out.extend_from_slice(&self.competing_confidence.get().to_le_bytes());
        put_bytes(out, self.flagged_by.as_bytes());
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
    /// This kind's discriminant byte (see the [module docs](self)).
    pub(crate) const TAG: u8 = 5;

    pub(crate) fn hash(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.factor.to_bits().to_le_bytes());
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
    /// This kind's discriminant byte (see the [module docs](self)).
    pub(crate) const TAG: u8 = 6;

    pub(crate) fn hash(&self, out: &mut Vec<u8>) {
        put_bytes(out, self.keyword.as_bytes());
        // The hint's location identifies where the keyword sits; its data
        // payload is auxiliary context and stays out of the hash.
        put_opt(
            out,
            self.hint.as_ref().map(|h| h.location.hash()).as_deref(),
        );
        put_opt(out, self.location.as_ref().map(|l| l.hash()).as_deref());
    }
}

/// An operator hid the entity.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Redaction {
    /// Which operator (name + version) ran.
    pub operator: OperatorId,
    /// How much the output leaks about the original.
    pub leak_profile: LeakProfile,
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
    /// This kind's discriminant byte (see the [module docs](self)).
    pub(crate) const TAG: u8 = 7;

    pub(crate) fn hash(&self, out: &mut Vec<u8>) {
        put_bytes(out, self.operator.name.as_bytes());
        put_bytes(out, self.operator.version.as_bytes());
        out.push(self.leak_profile as u8);
        put_opt(out, self.key_id.as_ref().map(|s| s.as_bytes()));
        self.matched_by.hash(out);
        hash_opt_attribution(out, self.attribution.as_ref());
        put_opt(
            out,
            self.span_hash.as_ref().map(|h| h.as_bytes().as_slice()),
        );
        match self.span_length {
            Some(length) => {
                out.push(1);
                out.extend_from_slice(&length.to_le_bytes());
            }
            None => out.push(0),
        }
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
    /// This kind's discriminant byte (see the [module docs](self)).
    pub(crate) const TAG: u8 = 8;

    pub(crate) fn hash(&self, out: &mut Vec<u8>) {
        out.push(self.intent as u8);
        put_bytes(out, &self.location.hash());
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
    /// This kind's discriminant byte (see the [module docs](self)).
    pub(crate) const TAG: u8 = 9;

    pub(crate) fn hash(&self, out: &mut Vec<u8>) {
        put_bytes(out, self.operator.name.as_bytes());
        put_bytes(out, self.operator.version.as_bytes());
        self.matched_by.hash(out);
        hash_opt_attribution(out, self.attribution.as_ref());
    }
}

/// Fold an optional [`Attribution`] into `out`: a presence byte, then the
/// attribution's own bytes if present. Shared by [`Redaction`] and [`Selection`].
fn hash_opt_attribution(out: &mut Vec<u8>, attribution: Option<&Attribution>) {
    match attribution {
        Some(attribution) => {
            out.push(1);
            attribution.hash(out);
        }
        None => out.push(0),
    }
}

/// Metadata of a pattern/dictionary recognition, carried by [`Pattern`].
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct PatternEvent {
    /// Name of the pattern that matched (e.g. `"ssn"`, `"email"`).
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub name: HipStr<'static>,
    /// Literal regex source that matched, when exposed.
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub regex: Option<HipStr<'static>>,
    /// Name of the validator that confirmed the match (e.g. `"luhn"`).
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub validator: Option<HipStr<'static>>,
    /// Whether contextual analysis (keyword co-occurrence) adjusted the score
    /// for this match.
    pub contextual: bool,
}

/// Metadata of a model/NER recognition, carried by [`Model`].
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ModelEvent {
    /// Model name (e.g. `"spacy-en-core-web-lg"`, `"gpt-4"`).
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub name: HipStr<'static>,
    /// Model version string, when known.
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub version: Option<HipStr<'static>>,
    /// Whether contextual analysis adjusted the score for this match.
    pub contextual: bool,
}
