//! A single [`AuditEvent`] in an entity's life, and the [`AuditKind`] of
//! event it can be.

use hipstr::HipStr;
use jiff::Timestamp;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::payload::{
    Calibration, Conflict, Contested, Deduplication, KindPayload, Manual, Model, ModelEvent,
    Pattern, PatternEvent, Redaction, Refinement, Selection,
};
use super::{Attribution, AuditHash, RuleMatch};
use crate::entity::LabelRef;
use crate::modality::{Hint, Modality};
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
            AuditKind::Pattern(Pattern { location, pattern }),
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
            AuditKind::Model(Model { location, model }),
        )
    }

    /// Deduplication (fusion) event combining several detections. Its
    /// `confidence` is the pooled score of the fused entities.
    pub fn deduplication(strategy: impl Into<HipStr<'static>>, confidence: Confidence) -> Self {
        let strategy = strategy.into();
        Self::unlinked(
            strategy.clone(),
            confidence,
            AuditKind::Deduplication(Deduplication { strategy }),
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
            AuditKind::Conflict(Conflict {
                competing_label,
                competing_confidence,
                resolved_by,
            }),
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
            AuditKind::Contested(Contested {
                competing_label,
                competing_confidence,
                flagged_by,
            }),
        )
    }

    /// Calibration event rescaling confidence by `factor`. `confidence` is the
    /// score after rescaling.
    pub fn calibration(confidence: Confidence, factor: f64) -> Self {
        Self::unlinked(
            HipStr::borrowed("calibration"),
            confidence,
            AuditKind::Calibration(Calibration { factor }),
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
            AuditKind::Refinement(Refinement {
                keyword: keyword.into(),
                hint,
                location,
            }),
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
            AuditKind::Redaction(Redaction {
                operator,
                leak_profile,
                key_id: None,
                matched_by,
                attribution,
                span_hash: None,
                span_length: None,
            }),
        )
    }

    /// The redaction *decision*: `operator` was picked to hide the entity (at
    /// the entity's `confidence`), recorded before it is applied so the pick
    /// can be reviewed. The [`redaction`](Self::redaction) event that follows
    /// records the operator actually run.
    pub fn selection(
        operator: OperatorId,
        confidence: Confidence,
        matched_by: RuleMatch,
        attribution: Option<Attribution>,
    ) -> Self {
        let source = operator.name.clone();
        Self::unlinked(
            source,
            confidence,
            AuditKind::Selection(Selection {
                operator,
                matched_by,
                attribution,
            }),
        )
    }

    /// A human override at `location` with the entity's asserted `confidence`
    /// — the birth event of a manually-added entity, or the marker recorded
    /// when a reviewer suppresses a detected one. Attach the rationale and the
    /// actor with [`with_reason`] / [`with_actor`]; the `source` mirrors the
    /// actor, or is `"manual"` when none is given.
    ///
    /// [`with_reason`]: Self::with_reason
    /// [`with_actor`]: Self::with_actor
    pub fn manual(location: M::Location, confidence: Confidence) -> Self {
        Self::unlinked(
            HipStr::borrowed("manual"),
            confidence,
            AuditKind::Manual(Manual {
                location,
                reason: None,
                actor: None,
            }),
        )
    }

    /// Set the rationale on a [`manual`](Self::manual) event (a no-op on any
    /// other kind).
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<HipStr<'static>>) -> Self {
        if let AuditKind::Manual(manual) = &mut self.kind {
            manual.reason = Some(reason.into());
        }
        self
    }

    /// Set the actor on a [`manual`](Self::manual) event (a no-op on any other
    /// kind); the actor also becomes the event's `source`.
    #[must_use]
    pub fn with_actor(mut self, actor: impl Into<HipStr<'static>>) -> Self {
        let actor = actor.into();
        if let AuditKind::Manual(manual) = &mut self.kind {
            self.source = actor.clone();
            manual.actor = Some(actor);
        }
        self
    }

    /// Whether this event is a recognition (pattern or model).
    pub fn is_recognition(&self) -> bool {
        matches!(self.kind, AuditKind::Pattern(_) | AuditKind::Model(_))
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
pub(super) fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(bytes);
}

/// Append an optional byte string: a presence byte, then the value if present.
pub(super) fn put_opt(out: &mut Vec<u8>, value: Option<&[u8]>) {
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
        // Each arm writes its payload's central discriminant [`TAG`], then folds
        // the payload's own fields in. The per-kind logic lives with the kind's
        // struct (in [`payload`](super::payload)); here it is uniform dispatch.
        match self {
            Self::Pattern(p) => tagged(out, p),
            Self::Model(m) => tagged(out, m),
            Self::Deduplication(d) => tagged(out, d),
            Self::Conflict(c) => tagged(out, c),
            Self::Contested(c) => tagged(out, c),
            Self::Calibration(c) => tagged(out, c),
            Self::Refinement(r) => tagged(out, r),
            Self::Redaction(r) => tagged(out, r),
            Self::Selection(s) => tagged(out, s),
            Self::Manual(m) => tagged(out, m),
        }
    }
}

/// Write a payload's discriminant [`TAG`](KindPayload::TAG) then its fields —
/// the uniform "tag, then body" every [`AuditKind`] arm folds into the hash.
fn tagged<P: KindPayload>(out: &mut Vec<u8>, payload: &P) {
    out.push(P::TAG);
    payload.hash_into(out);
}

/// Kind of an [`AuditEvent`], carrying its event-specific detail and the
/// rationale for why it happened.
///
/// A thin tagged union: each variant wraps one payload struct (e.g.
/// [`Redaction`], [`Selection`], [`Manual`]) that owns that kind's fields, its
/// docs, and how it folds into the audit hash. Match on a variant to reach its
/// payload:
///
/// ```
/// # use elide_core::entity::audit::AuditKind;
/// # use elide_core::modality::text::Text;
/// # fn show(kind: &AuditKind<Text>) {
/// if let AuditKind::Redaction(redaction) = kind {
///     let _ = &redaction.operator;
/// }
/// # }
/// ```
///
/// `#[non_exhaustive]`: new event kinds (verification, annotation, …) can be
/// added compatibly. The recognition kinds ([`Pattern`], [`Model`]) carry the
/// matched [`Location`]; the rest carry their own data.
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
        content = "detail",
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
    Pattern(Pattern<M>),
    /// A model / NER recognizer matched here.
    Model(Model<M>),
    /// Several detections were fused into one entity.
    Deduplication(Deduplication),
    /// A competing detection of a different label over the same span was
    /// resolved against this (winning) entity.
    Conflict(Conflict),
    /// A competing detection of a different label over the same span was left
    /// *unresolved*: both entities survive, flagged for a human to settle.
    Contested(Contested),
    /// The entity's confidence was rescaled by a per-recognizer factor.
    Calibration(Calibration),
    /// A context keyword near the entity lifted its confidence.
    Refinement(Refinement<M>),
    /// An operator hid the entity.
    Redaction(Redaction),
    /// An operator was *picked* to hide the entity — the redaction decision,
    /// recorded before it is applied so it can be reviewed first.
    Selection(Selection),
    /// A human override, outside automatic detection: an entity a reviewer
    /// added by hand, or a detected one they marked to ignore.
    Manual(Manual<M>),
}
