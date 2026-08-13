//! A single [`AuditEvent`] in an entity's life, and the [`AuditKind`] of
//! event it can be.

use hipstr::HipStr;
use jiff::Timestamp;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{Attribution, AuditHash, RuleMatch};
use crate::entity::LabelRef;
use crate::modality::{Hint, Modality, ModalityLocation};
use crate::operator::{LeakProfile, OperatorId};
use crate::primitive::Confidence;

/// One node in an entity's audit DAG: a thing that happened, with its
/// effect on confidence and its tamper-evident links.
///
/// Events are recorded on an entity's [`AuditLog`], forming the full audit
/// trail of its life: each recognizer that found it, the deduplication that
/// fused them, any score calibration, and the redaction that hid it. The
/// uniform spine (who, resulting score, when) is the same for every event; the
/// [`kind`] carries the event-specific detail *and* the explanation of why the
/// event happened.
///
/// Each event links to its [`parents`] by their [`hash`]: a birth event (a
/// recognizer's first detection) has no parents, a normal step has one, and a
/// [fusion](crate::entity::audit::AuditLog::record_fusion) has several. Its own
/// [`hash`] folds the
/// payload together with the parents' hashes, so altering any event breaks
/// every event downstream of it: [`AuditLog::verify`] walks the DAG and
/// reports the first break.
///
/// `entity.confidence` always equals the [`confidence`] of the most recent
/// event. The confidence *flowing in* is not stored: it is the parents'
/// [`confidence`], recovered from the DAG.
///
/// [`AuditLog`]: crate::entity::audit::AuditLog
/// [`AuditLog::record_fusion`]: crate::entity::audit::AuditLog::record_fusion
/// [`AuditLog::verify`]: crate::entity::audit::AuditLog::verify
/// [`kind`]: AuditEvent::kind
/// [`parents`]: AuditEvent::parents
/// [`hash`]: AuditEvent::hash
/// [`confidence`]: AuditEvent::confidence
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
        rename = "{M}AuditEvent"
    )
)]
pub struct AuditEvent<M: Modality> {
    /// Who produced this event: a recognizer name, a deduplication strategy,
    /// an operator, or whatever acted.
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub source: HipStr<'static>,
    /// Confidence after this event: the entity's effective confidence once it
    /// has happened.
    pub confidence: Confidence,
    /// When the event happened (UTC).
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub timestamp: Timestamp,
    /// Kind of event, with its event-specific detail and its rationale.
    pub kind: AuditKind<M>,
    /// The events this one follows, by their hash. Empty for a birth event, one
    /// entry for a normal step, several for a fusion. Assigned by [`AuditLog`]
    /// when the event is recorded; read it through [`parents`](Self::parents).
    ///
    /// Not publicly settable: the links are the tamper-evident structure, so
    /// only [`AuditLog::record`] / [`AuditLog::record_fusion`] assign them.
    ///
    /// [`AuditLog`]: crate::entity::audit::AuditLog
    /// [`AuditLog::record`]: crate::entity::audit::AuditLog::record
    /// [`AuditLog::record_fusion`]: crate::entity::audit::AuditLog::record_fusion
    pub(super) parents: Vec<AuditHash>,
    /// This event's hash, over its payload and its parents' hashes. Assigned by
    /// [`AuditLog`] when the event is recorded; read it through
    /// [`hash`](Self::hash). Recomputing it is how the DAG is verified.
    ///
    /// Not publicly settable: only [`AuditLog`] assigns it, so a caller cannot
    /// forge a self-consistent event outside the recording path.
    ///
    /// [`AuditLog`]: crate::entity::audit::AuditLog
    pub(super) hash: AuditHash,
}

impl<M: Modality> AuditEvent<M> {
    /// Assemble an unlinked event: its payload only. [`parents`] is empty and
    /// [`hash`] is [`GENESIS`] until [`AuditLog`] records it and assigns the
    /// links.
    ///
    /// [`parents`]: Self::parents
    /// [`hash`]: Self::hash
    /// [`GENESIS`]: AuditHash::GENESIS
    /// [`AuditLog`]: crate::entity::audit::AuditLog
    fn unlinked(source: HipStr<'static>, confidence: Confidence, kind: AuditKind<M>) -> Self {
        Self {
            source,
            confidence,
            timestamp: Timestamp::now(),
            kind,
            parents: Vec::new(),
            hash: AuditHash::GENESIS,
        }
    }

    /// Recognition event from a pattern/dictionary recognizer.
    pub fn pattern(
        source: impl Into<HipStr<'static>>,
        confidence: Confidence,
        location: M::Location,
        pattern: PatternEvent,
    ) -> Self {
        Self::unlinked(
            source.into(),
            confidence,
            AuditKind::Pattern { location, pattern },
        )
    }

    /// Recognition event from a model/NER recognizer.
    pub fn model(
        source: impl Into<HipStr<'static>>,
        confidence: Confidence,
        location: M::Location,
        model: ModelEvent,
    ) -> Self {
        Self::unlinked(
            source.into(),
            confidence,
            AuditKind::Model { location, model },
        )
    }

    /// Deduplication (fusion) event combining several detections. Its
    /// `confidence` is the pooled score of the fused entities.
    pub fn deduplication(strategy: impl Into<HipStr<'static>>, confidence: Confidence) -> Self {
        let strategy = strategy.into();
        Self::unlinked(
            strategy.clone(),
            confidence,
            AuditKind::Deduplication { strategy },
        )
    }

    /// Conflict-resolution event: a competing detection of a *different*
    /// label over the same span was arbitrated against this entity, which
    /// won. The loser is recorded here rather than dropped silently.
    ///
    /// The winner's confidence is unchanged (resolution does not rescale it).
    pub fn conflict(
        resolved_by: impl Into<HipStr<'static>>,
        confidence: Confidence,
        competing_label: LabelRef,
        competing_confidence: Confidence,
    ) -> Self {
        let resolved_by = resolved_by.into();
        Self::unlinked(
            resolved_by.clone(),
            confidence,
            AuditKind::Conflict {
                competing_label,
                competing_confidence,
                resolved_by,
            },
        )
    }

    /// Contested event: a cross-label overlap left unresolved for human
    /// review. Recorded on each entity of the pair, naming the other. The
    /// entity's confidence is unchanged.
    pub fn contested(
        flagged_by: impl Into<HipStr<'static>>,
        confidence: Confidence,
        competing_label: LabelRef,
        competing_confidence: Confidence,
    ) -> Self {
        let flagged_by = flagged_by.into();
        Self::unlinked(
            flagged_by.clone(),
            confidence,
            AuditKind::Contested {
                competing_label,
                competing_confidence,
                flagged_by,
            },
        )
    }

    /// Calibration event rescaling confidence by `factor`. `confidence` is the
    /// score after rescaling.
    pub fn calibration(confidence: Confidence, factor: f64) -> Self {
        Self::unlinked(
            HipStr::borrowed("calibration"),
            confidence,
            AuditKind::Calibration { factor },
        )
    }

    /// Refinement event: a context keyword near the entity lifted its
    /// confidence to `confidence`.
    ///
    /// `location` is where the boosting keyword sits in the medium: for a
    /// hint match the hint's own location, for an in-text-window match the
    /// keyword resolved through the modality (`None` if it couldn't be
    /// placed).
    pub fn refinement(
        source: impl Into<HipStr<'static>>,
        confidence: Confidence,
        keyword: impl Into<HipStr<'static>>,
        hint: Option<Hint<M>>,
        location: Option<M::Location>,
    ) -> Self {
        Self::unlinked(
            source.into(),
            confidence,
            AuditKind::Refinement {
                keyword: keyword.into(),
                hint,
                location,
            },
        )
    }

    /// Redaction event hiding the entity with `operator`. The entity's
    /// confidence is unchanged.
    pub fn redaction(
        operator: OperatorId,
        leak_profile: LeakProfile,
        confidence: Confidence,
        matched_by: RuleMatch,
        attribution: Option<Attribution>,
    ) -> Self {
        let source = operator.name.clone();
        Self::unlinked(
            source,
            confidence,
            AuditKind::Redaction {
                operator,
                leak_profile,
                key_id: None,
                matched_by,
                attribution,
                span_hash: None,
                span_length: None,
            },
        )
    }

    /// Whether this event is a recognition (pattern or model).
    pub fn is_recognition(&self) -> bool {
        matches!(
            self.kind,
            AuditKind::Pattern { .. } | AuditKind::Model { .. }
        )
    }

    /// The events this one follows, by their [`hash`](Self::hash): none for a
    /// birth event, one for a normal step, several for a fusion.
    pub fn parents(&self) -> &[AuditHash] {
        &self.parents
    }

    /// This event's tamper-evident hash, over its payload and its parents.
    pub fn hash(&self) -> AuditHash {
        self.hash
    }
}

/// Append a length-prefixed byte string, so concatenated fields cannot be
/// confused across a boundary (e.g. `"ab" + "c"` differs from `"a" + "bc"`).
fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(bytes);
}

/// Append an optional byte string: a presence byte, then the value if present.
fn put_opt(out: &mut Vec<u8>, value: Option<&[u8]>) {
    match value {
        Some(bytes) => {
            out.push(1);
            put_bytes(out, bytes);
        }
        None => out.push(0),
    }
}

impl<M: Modality> AuditKind<M> {
    /// This event kind's identifying bytes, for the audit hash.
    ///
    /// A leading discriminant byte tags the variant, then each field is folded
    /// in (locations via [`ModalityLocation::hash`]). No serialization and no
    /// `serde` bound: the tamper-evident chain hashes the same in every build.
    ///
    /// [`ModalityLocation::hash`]: crate::modality::ModalityLocation::hash
    pub(crate) fn hash(&self, out: &mut Vec<u8>) {
        match self {
            Self::Pattern { location, pattern } => {
                out.push(0);
                put_bytes(out, &location.hash());
                put_bytes(out, pattern.name.as_bytes());
                put_opt(out, pattern.regex.as_ref().map(|s| s.as_bytes()));
                put_opt(out, pattern.validator.as_ref().map(|s| s.as_bytes()));
                out.push(pattern.contextual.into());
            }
            Self::Model { location, model } => {
                out.push(1);
                put_bytes(out, &location.hash());
                put_bytes(out, model.name.as_bytes());
                put_opt(out, model.version.as_ref().map(|s| s.as_bytes()));
                out.push(model.contextual.into());
            }
            Self::Deduplication { strategy } => {
                out.push(2);
                put_bytes(out, strategy.as_bytes());
            }
            Self::Conflict {
                competing_label,
                competing_confidence,
                resolved_by,
            } => {
                out.push(3);
                put_bytes(out, competing_label.as_str().as_bytes());
                out.extend_from_slice(&competing_confidence.get().to_le_bytes());
                put_bytes(out, resolved_by.as_bytes());
            }
            Self::Contested {
                competing_label,
                competing_confidence,
                flagged_by,
            } => {
                out.push(4);
                put_bytes(out, competing_label.as_str().as_bytes());
                out.extend_from_slice(&competing_confidence.get().to_le_bytes());
                put_bytes(out, flagged_by.as_bytes());
            }
            Self::Calibration { factor } => {
                out.push(5);
                out.extend_from_slice(&factor.to_bits().to_le_bytes());
            }
            Self::Refinement {
                keyword,
                hint,
                location,
            } => {
                out.push(6);
                put_bytes(out, keyword.as_bytes());
                // The hint's location identifies where the keyword sits; its
                // data payload is auxiliary context and stays out of the hash.
                put_opt(out, hint.as_ref().map(|h| h.location.hash()).as_deref());
                put_opt(out, location.as_ref().map(|l| l.hash()).as_deref());
            }
            Self::Redaction {
                operator,
                leak_profile,
                key_id,
                matched_by,
                attribution,
                span_hash,
                span_length,
            } => {
                out.push(7);
                put_bytes(out, operator.name.as_bytes());
                put_bytes(out, operator.version.as_bytes());
                out.push(*leak_profile as u8);
                put_opt(out, key_id.as_ref().map(|s| s.as_bytes()));
                matched_by.hash(out);
                match attribution {
                    Some(attribution) => {
                        out.push(1);
                        put_bytes(out, attribution.name.as_bytes());
                        put_opt(out, attribution.description.as_ref().map(|s| s.as_bytes()));
                    }
                    None => out.push(0),
                }
                put_opt(out, span_hash.as_ref().map(|h| h.as_bytes().as_slice()));
                match span_length {
                    Some(length) => {
                        out.push(1);
                        out.extend_from_slice(&length.to_le_bytes());
                    }
                    None => out.push(0),
                }
            }
        }
    }
}

/// Kind of an [`AuditEvent`], carrying its event-specific detail and the
/// rationale for why it happened.
///
/// `#[non_exhaustive]`: new event kinds (verification, annotation, …)
/// can be added compatibly. The recognition kinds ([`Pattern`],
/// [`Model`]) carry the matched [`Location`]; the rest carry their own
/// data.
///
/// [`Pattern`]: AuditKind::Pattern
/// [`Model`]: AuditKind::Model
/// [`Location`]: Modality::Location
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(
        tag = "kind",
        rename_all = "snake_case",
        bound = "M::Location: Serialize + for<'a> Deserialize<'a>, \
                 M::Data: Serialize + for<'a> Deserialize<'a>"
    )
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        bound = "M: schemars::JsonSchema, M::Location: schemars::JsonSchema, M::Data: schemars::JsonSchema",
        rename = "{M}AuditKind"
    )
)]
#[non_exhaustive]
pub enum AuditKind<M: Modality> {
    /// A pattern or dictionary recognizer matched here.
    Pattern {
        /// Where the recognizer matched.
        location: M::Location,
        /// Pattern detail.
        pattern: PatternEvent,
    },
    /// A model / NER recognizer matched here.
    Model {
        /// Where the recognizer matched.
        location: M::Location,
        /// Model detail.
        model: ModelEvent,
    },
    /// Several detections were fused into one entity.
    Deduplication {
        /// Name of the fusion strategy that combined them.
        #[cfg_attr(feature = "schema", schemars(with = "String"))]
        strategy: HipStr<'static>,
    },
    /// A competing detection of a different label over the same span was
    /// resolved against this (winning) entity.
    Conflict {
        /// The label of the detection that lost arbitration.
        competing_label: LabelRef,
        /// The loser's confidence at resolution time.
        competing_confidence: Confidence,
        /// Name of the conflict policy that chose the winner.
        #[cfg_attr(feature = "schema", schemars(with = "String"))]
        resolved_by: HipStr<'static>,
    },
    /// A competing detection of a different label over the same span was left
    /// *unresolved*: both entities survive, flagged for a human to settle.
    /// Recorded on each entity of the contested pair, naming the other.
    Contested {
        /// The label of the competing detection.
        competing_label: LabelRef,
        /// The competing detection's confidence.
        competing_confidence: Confidence,
        /// Name of the policy that flagged the contest.
        #[cfg_attr(feature = "schema", schemars(with = "String"))]
        flagged_by: HipStr<'static>,
    },
    /// The entity's confidence was rescaled by a per-recognizer factor.
    Calibration {
        /// Multiplier applied.
        factor: f64,
    },
    /// A context keyword near the entity lifted its confidence.
    Refinement {
        /// Keyword that fired the boost.
        #[cfg_attr(feature = "schema", schemars(with = "String"))]
        keyword: HipStr<'static>,
        /// The located [`Hint`] the keyword fired from, when the match came
        /// from an out-of-band hint (a column header, a key) rather than
        /// the in-text word window. `None` for an in-text-window match.
        ///
        /// [`Hint`]: crate::modality::Hint
        hint: Option<Hint<M>>,
        /// Where the boosting keyword sits in the medium. For a hint match
        /// this mirrors the hint's own location; for an in-text-window match
        /// it is the keyword resolved through the modality's [`locate`] (a
        /// pixel box for image, a time span for audio, the byte range for
        /// text/tabular). `None` when the keyword's stream range could not be
        /// placed, symmetric with a match the recognizer itself drops.
        ///
        /// [`locate`]: crate::modality::TextRecognizable::locate
        location: Option<M::Location>,
    },
    /// An operator hid the entity.
    Redaction {
        /// Which operator (name + version) ran.
        operator: OperatorId,
        /// How much the output leaks about the original.
        leak_profile: LeakProfile,
        /// Identifier of the key needed to reverse it, if reversible.
        #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
        key_id: Option<HipStr<'static>>,
        /// Which selection rule chose this operator: the automatic "why"
        /// (matched a label, a tag, a predicate, or the fallback).
        matched_by: RuleMatch,
        /// The author-supplied policy rationale, when the operator carried an
        /// [`Attribution`]; `None` otherwise.
        attribution: Option<Attribution>,
        /// BLAKE3 digest of the original text the operator hid, when the
        /// redaction layer recorded it. Proves *what* was redacted without
        /// storing the plaintext; `None` when the operator did not capture it.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        span_hash: Option<AuditHash>,
        /// Byte length of the original text the operator hid, paired with
        /// [`span_hash`](Self::Redaction::span_hash). `None` when not captured.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        span_length: Option<u32>,
    },
}

/// Detail of a pattern/dictionary recognition.
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
    /// Whether contextual analysis (keyword co-occurrence) adjusted the
    /// score for this match.
    pub contextual: bool,
}

/// Detail of a model/NER recognition.
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
