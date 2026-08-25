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

use super::hash::AuditHasher;
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

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.bytes(&self.location.hash());
        out.bytes(self.pattern.name.as_bytes());
        out.opt(self.pattern.regex.as_ref().map(|s| s.as_bytes()));
        out.opt(self.pattern.validator.as_ref().map(|s| s.as_bytes()));
        out.byte(self.pattern.contextual.into());
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

    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        out.bytes(&self.location.hash());
        out.bytes(self.model.name.as_bytes());
        out.opt(self.model.version.as_ref().map(|s| s.as_bytes()));
        out.byte(self.model.contextual.into());
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
    /// This kind's discriminant byte (see the [module docs](self)).
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
    /// This kind's discriminant byte (see the [module docs](self)).
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
    /// This kind's discriminant byte (see the [module docs](self)).
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
    /// This kind's discriminant byte (see the [module docs](self)).
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
    /// This kind's discriminant byte (see the [module docs](self)).
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
