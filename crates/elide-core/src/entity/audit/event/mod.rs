//! A single [`AuditEvent`] in an entity's life: its spine (source, confidence,
//! timestamp), its [`AuditKind`], and its tamper-evident links.
//!
//! The per-kind detail lives in the [payload](self) submodules, grouped by
//! role: [`recognition`] (a recognizer matched), [`reconcile`] (detections were
//! combined and scored), and [`apply`] (the redaction decision and human
//! overrides). Each payload owns its fields, its `new` / `with_*` builders, its
//! central `TAG` discriminant, and how it folds into the hash.

mod kind;

/// Application payloads: the redaction decision and its execution, and human
/// overrides.
pub mod apply;
/// Recognition payloads: a pattern or model recognizer matched an entity.
pub mod recognition;
/// Reconciliation payloads: fusion, cross-label arbitration, calibration, and
/// context boosting.
pub mod reconcile;

use hipstr::HipStr;
use jiff::Timestamp;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub use self::apply::{Manual, ManualIntent, Redaction, Selection};
pub use self::kind::AuditKind;
pub use self::recognition::{Flagged, Model, ModelId, Pattern, PatternRule};
pub use self::reconcile::{Calibration, Conflict, Contested, Deduplication, Refinement};
use super::AuditHash;
use super::hash::AuditHasher;
use crate::modality::Modality;
use crate::primitive::Confidence;
use crate::redaction::HiddenContentKind;

/// One node of an entity's audit DAG: something that happened to the entity.
///
/// It records the effect on confidence and its tamper-evident links.
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
/// [`AuditLog::verify`]: crate::entity::audit::AuditLog::verify
/// [`kind`]: AuditEvent::kind
/// [`parents`]: AuditEvent::parents
/// [`hash`]: AuditEvent::hash
/// [`confidence`]: AuditEvent::confidence
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
    /// Assemble an event from its `source` (who produced it), the `confidence`
    /// after it, and its `kind`, pass a payload with `.into()` (e.g.
    /// `Redaction::new(..).into()`). [`parents`] is empty and [`hash`] is
    /// [`GENESIS`] until [`AuditLog`] records it and assigns the links.
    ///
    /// The kind-specific constructors ([`pattern`], [`redaction`], …) are
    /// convenience wrappers that derive `source` and build the kind for you;
    /// reach for `new` to set an explicit source (e.g. a reviewer's id on a
    /// [`manual`] override) or to build a kind the wrappers don't cover.
    ///
    /// [`parents`]: Self::parents
    /// [`hash`]: Self::hash
    /// [`GENESIS`]: AuditHash::GENESIS
    /// [`AuditLog`]: crate::entity::audit::AuditLog
    /// [`pattern`]: Self::pattern
    /// [`redaction`]: Self::redaction
    /// [`manual`]: Self::manual
    pub fn new(
        source: impl Into<HipStr<'static>>,
        confidence: Confidence,
        kind: impl Into<AuditKind<M>>,
    ) -> Self {
        Self {
            source: source.into(),
            confidence,
            timestamp: Timestamp::now(),
            kind: kind.into(),
            parents: Vec::new(),
            hash: AuditHash::GENESIS,
        }
    }

    /// This event's hash: BLAKE3 over its parents' hashes, its spine (source,
    /// timestamp, confidence), and its [`kind`](Self::kind) detail.
    ///
    /// The kind folds itself in through its own `hash_into`, which hashes
    /// each field directly (locations via [`ModalityLocation::hash`]), so no
    /// serialization and no `serde` bound is involved. Parents are folded in
    /// first, so an event's hash covers the exact sub-DAG it descends from.
    /// [`AuditLog`] assigns the result to [`hash`](Self::hash) when recording,
    /// and recomputes it here to verify the trail.
    ///
    /// [`ModalityLocation::hash`]: crate::modality::ModalityLocation::hash
    /// [`AuditLog`]: crate::entity::audit::AuditLog
    pub(super) fn digest(&self) -> AuditHash {
        let mut hasher = AuditHasher::new();
        for parent in &self.parents {
            hasher.raw(parent.as_bytes());
        }
        hasher.raw(&(self.parents.len() as u64).to_le_bytes());
        // `source` is variable-width, so length-prefix it (`bytes`), the
        // fixed-width spine fields around it use `raw`.
        hasher.bytes(self.source.as_bytes());
        hasher.raw(&self.timestamp.as_nanosecond().to_le_bytes());
        hasher.raw(&self.confidence.get().to_le_bytes());
        self.kind.hash_into(&mut hasher);
        hasher.finish()
    }

    /// A recognition event from a pattern or dictionary recognizer's match.
    ///
    /// The `source` recognizer's `rule` matched at `location`.
    pub fn pattern(
        source: impl Into<HipStr<'static>>,
        confidence: Confidence,
        location: M::Location,
        rule: PatternRule,
    ) -> Self {
        Self::new(source, confidence, Pattern { location, rule })
    }

    /// A recognition event from a model or NER recognizer's match.
    ///
    /// The `source` recognizer matched at `location` with `model`.
    pub fn model(
        source: impl Into<HipStr<'static>>,
        confidence: Confidence,
        location: M::Location,
        model: ModelId,
    ) -> Self {
        Self::new(source, confidence, Model { location, model })
    }

    /// A flagged item: the `source` format labels an item of hidden content of
    /// kind `content` at `location`, flagged as a redaction subject. Unlike
    /// [`pattern`](Self::pattern) / [`model`](Self::model), the item is present
    /// rather than matched, so it carries its own event kind.
    pub fn flagged(
        source: impl Into<HipStr<'static>>,
        confidence: Confidence,
        location: M::Location,
        content: HiddenContentKind,
    ) -> Self {
        Self::new(source, confidence, Flagged { location, content })
    }

    /// Deduplication (fusion) event combining several detections. Its
    /// `confidence` is the pooled score of the fused entities; the `source` is
    /// the fusion strategy's name.
    pub fn deduplication(strategy: impl Into<HipStr<'static>>, confidence: Confidence) -> Self {
        let strategy = strategy.into();
        Self::new(strategy.clone(), confidence, Deduplication { strategy })
    }

    /// Conflict-resolution event: a competing detection was arbitrated against
    /// this entity, which won. The loser is recorded in `conflict` rather than
    /// dropped silently; `confidence` is the winner's own (unchanged) score. The
    /// `source` is the conflict payload's `resolved_by`.
    pub fn conflict(conflict: Conflict, confidence: Confidence) -> Self {
        Self::new(conflict.resolved_by.clone(), confidence, conflict)
    }

    /// Contested event: a cross-label overlap left unresolved for human review,
    /// recorded on each entity of the pair naming the other in `contested`.
    /// `confidence` is this entity's own (unchanged) score; the `source` is the
    /// payload's `flagged_by`.
    pub fn contested(contested: Contested, confidence: Confidence) -> Self {
        Self::new(contested.flagged_by.clone(), confidence, contested)
    }

    /// Calibration event rescaling confidence by `factor`. `confidence` is the
    /// score after rescaling.
    pub fn calibration(confidence: Confidence, factor: f64) -> Self {
        Self::new(
            HipStr::borrowed("calibration"),
            confidence,
            Calibration { factor },
        )
    }

    /// Refinement event: a context keyword near the entity lifted its confidence
    /// to `confidence`, produced by the `source` recognizer. Build the
    /// `refinement` payload with its keyword and (optionally) hint / location.
    pub fn refinement(
        source: impl Into<HipStr<'static>>,
        confidence: Confidence,
        refinement: Refinement<M>,
    ) -> Self {
        Self::new(source, confidence, refinement)
    }

    /// A redaction event: the `source` operator applied a selection.
    ///
    /// The selection `redaction` names is the one applied; the entity's
    /// `confidence` is unchanged.
    pub fn redaction(
        source: impl Into<HipStr<'static>>,
        redaction: Redaction,
        confidence: Confidence,
    ) -> Self {
        Self::new(source, confidence, redaction)
    }

    /// The redaction *decision*: the operator in `selection` was picked to hide
    /// the entity (at its `confidence`), recorded before it is applied so the
    /// pick can be reviewed. The [`redaction`](Self::redaction) event that
    /// applies it names this event by its hash. The `source` is the operator's
    /// name.
    pub fn selection(selection: Selection, confidence: Confidence) -> Self {
        Self::new(selection.operator.name.clone(), confidence, selection)
    }

    /// A human override recording `manual` at the entity's asserted
    /// `confidence`.
    ///
    /// The `manual` detail is built with its intent, location, and optional
    /// [`Attribution`]. The `source` names the reviewer who made the override:
    /// pass their id, or `"manual"` when unattributed. To record one on an
    /// entity, use `Entity::record_manual`, which takes the entity's own
    /// confidence.
    ///
    /// [`Attribution`]: crate::entity::audit::Attribution
    pub fn manual(
        source: impl Into<HipStr<'static>>,
        confidence: Confidence,
        manual: Manual<M>,
    ) -> Self {
        Self::new(source, confidence, manual)
    }

    /// Whether this event is a recognition (pattern, model, or flagged item).
    pub fn is_recognition(&self) -> bool {
        matches!(
            self.kind,
            AuditKind::Pattern(_) | AuditKind::Model(_) | AuditKind::Flagged(_)
        )
    }

    /// The events this one follows, by their [`hash`](Self::hash).
    ///
    /// None for a birth event, one for a normal step, several for a fusion.
    pub fn parents(&self) -> &[AuditHash] {
        &self.parents
    }

    /// This event's tamper-evident hash, over its payload and its parents.
    pub fn hash(&self) -> AuditHash {
        self.hash
    }
}
