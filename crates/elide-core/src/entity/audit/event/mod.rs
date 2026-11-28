//! A single [`AuditEvent`] in an entity's life: its spine (confidence,
//! timestamp), its [`AuditKind`], and its tamper-evident links.
//!
//! The per-kind detail lives in the [payload](self) submodules, grouped by
//! role: [`recognition`] (a recognizer matched), [`reconcile`] (detections were
//! combined and scored), and [`apply`] (the redaction decision and a reviewer's
//! work). Each payload owns its fields, its `new` / `with_*` builders, its
//! central `TAG` discriminant, and how it folds into the hash. Who acted is a
//! payload field typed to what can act there: a component for a recognition,
//! a reconciliation, or an operator's pick, a reviewer for their decisions.

mod kind;

/// Application payloads: the redaction decision and its execution, and a
/// reviewer's inclusions and decisions.
pub mod apply;
/// Recognition payloads: a pattern or model recognizer matched an entity.
pub mod recognition;
/// Reconciliation payloads: fusion, cross-label arbitration, calibration, and
/// context boosting.
pub mod reconcile;

use jiff::Timestamp;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub use self::apply::{Included, Manual, ManualIntent, Redacted, Selected};
pub use self::kind::AuditKind;
pub use self::recognition::{Annotated, Flagged, Model, ModelId, Pattern, PatternRule};
pub use self::reconcile::{Calibrated, Contested, Dropped, Fused, Refined, Resolved};
use super::AuditHash;
use super::hash::AuditHasher;
use crate::modality::Modality;
use crate::primitive::{ComponentId, Confidence};

/// One node of an entity's audit DAG: something that happened to the entity.
///
/// It records the effect on confidence and its tamper-evident links.
///
/// Events are recorded on an entity's [`AuditLog`], forming the full audit
/// trail of its life: each recognizer that found it, the fusion that
/// joined them, any score calibration, and the redaction that hid it. The
/// uniform spine (resulting score, when) is the same for every event; the
/// [`kind`] carries the event-specific detail, who acted (read whatever the
/// kind with [`actor`]), *and* the explanation of why the event happened.
///
/// Each event links to its [`parents`] by their [`hash`]: a birth event (a
/// recognition or a reviewer's inclusion) has no parents, a normal step has
/// one, and a [fusion](crate::entity::Entity::fuse) has several. Its own
/// [`hash`] folds the payload together with the parents' hashes, so altering
/// any event breaks every event downstream of it: [`AuditLog::verify`] walks
/// the DAG and reports the first break.
///
/// `entity.confidence` always equals the [`confidence`] of the most recent
/// event. The confidence *flowing in* is not stored: it is the parents'
/// [`confidence`], recovered from the DAG.
///
/// [`AuditLog`]: crate::entity::audit::AuditLog
/// [`AuditLog::verify`]: crate::entity::audit::AuditLog::verify
/// [`kind`]: AuditEvent::kind
/// [`actor`]: AuditEvent::actor
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
    /// Confidence after this event: the entity's effective confidence once it
    /// has happened.
    pub confidence: Confidence,
    /// When the event happened (UTC).
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub timestamp: Timestamp,
    /// Kind of event, with its event-specific detail, who acted, and its
    /// rationale.
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

/// Who acted in an [`AuditEvent`], read from its payload by
/// [`AuditEvent::actor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor<'a> {
    /// A component: a recognizer, a stream that flagged hidden content, a
    /// layer, a scoring, a reconciler, or an operator.
    Component(&'a ComponentId),
    /// A reviewer, by the id they gave.
    Reviewer(&'a str),
}

impl<M: Modality> AuditEvent<M> {
    /// An event of `kind` with the `confidence` after it, happening now; pass a
    /// payload with `.into()` (e.g. `Redacted::new(..).into()`), or as it is.
    /// [`parents`] is empty and [`hash`] is [`GENESIS`] until [`AuditLog`]
    /// records it and assigns the links.
    ///
    /// [`parents`]: Self::parents
    /// [`hash`]: Self::hash
    /// [`GENESIS`]: AuditHash::GENESIS
    /// [`AuditLog`]: crate::entity::audit::AuditLog
    pub fn new(confidence: Confidence, kind: impl Into<AuditKind<M>>) -> Self {
        Self {
            confidence,
            timestamp: Timestamp::now(),
            kind: kind.into(),
            parents: Vec::new(),
            hash: AuditHash::GENESIS,
        }
    }

    /// This event's hash: BLAKE3 over its parents' hashes, its spine
    /// (timestamp, confidence), and its [`kind`](Self::kind) detail.
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
        hasher.raw(&self.timestamp.as_nanosecond().to_le_bytes());
        hasher.raw(&self.confidence.get().to_le_bytes());
        self.kind.hash_into(&mut hasher);
        hasher.finish()
    }

    /// Who acted, as the payload names them: the component, or the reviewer.
    /// `None` for an [annotation](AuditKind::Annotated), which records a fact
    /// rather than an act, and for a [redaction](AuditKind::Redacted), whose
    /// operator is the one its selection names, see
    /// [`AuditLog::applied_selection`].
    ///
    /// [`AuditLog::applied_selection`]: crate::entity::audit::AuditLog::applied_selection
    pub fn actor(&self) -> Option<Actor<'_>> {
        let component = match &self.kind {
            AuditKind::Pattern(p) => &p.recognizer,
            AuditKind::Model(m) => &m.recognizer,
            AuditKind::Flagged(f) => &f.flagged_by,
            AuditKind::Fused(f) => &f.strategy,
            AuditKind::Resolved(r) => &r.resolved_by,
            AuditKind::Contested(c) => &c.flagged_by,
            AuditKind::Dropped(d) => &d.by,
            AuditKind::Calibrated(c) => &c.recognizer,
            AuditKind::Refined(r) => &r.recognizer,
            AuditKind::Selected(s) => &s.operator,
            AuditKind::Included(i) => return Some(Actor::Reviewer(&i.by)),
            AuditKind::Manual(m) => return Some(Actor::Reviewer(&m.by)),
            AuditKind::Redacted(_) | AuditKind::Annotated(_) => return None,
        };
        Some(Actor::Component(component))
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
