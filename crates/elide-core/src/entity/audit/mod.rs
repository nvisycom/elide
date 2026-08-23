//! The per-entity [`AuditLog`]: a tamper-evident DAG of [`AuditEvent`]s.
//!
//! [`AuditLog`]: crate::entity::audit::AuditLog

mod attribution;
mod event;
mod hash;
mod payload;
mod rule_match;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub use self::attribution::{Attribution, AttributionKind};
pub use self::event::{AuditEvent, AuditKind};
pub use self::hash::AuditHash;
pub use self::payload::{
    Calibration, Conflict, Contested, Deduplication, Manual, Model, ModelEvent, Pattern,
    PatternEvent, Redaction, Refinement, Selection,
};
pub use self::rule_match::RuleMatch;
use crate::modality::Modality;
use crate::primitive::Confidence;
use crate::{Error, ErrorKind, Result};

/// Full audit trail of an [`Entity`]: every [`AuditEvent`] in its life, as a
/// tamper-evident DAG.
///
/// Where Presidio keeps a shallow, optional, per-stage explanation that is
/// stripped by default, an `AuditLog` is always present and records the
/// entity's *entire* life: each recognizer that found it, the deduplication
/// that fused them, any confidence calibration, and the redaction that hid it.
/// Nothing is collapsed: every recognizer keeps its own recognition event with
/// its location and score.
///
/// The events form a **directed acyclic graph**, not a flat list. A recognizer
/// records a birth event (no parents); each later step links to the event it
/// follows; and a [fusion](Self::record_fusion) links to *several* parents at
/// once: the heads of the trails it combines. This is the true shape of a
/// deduplicated entity: two recognizers are siblings, then a fusion joins them.
/// The events are stored in the order they were recorded, which is a
/// topological order of the DAG (a parent is always recorded before its
/// children).
///
/// Two chains ride the DAG's edges:
///
/// - a **confidence chain**, where each event's [`confidence`] is the entity's
///   effective score after it; the score flowing *in* is its parents'
///   confidence, so [`final_confidence`] and the full history are recoverable;
/// - a **hash chain**, where each event's [`hash`] folds its payload together
///   with its parents' hashes, so any edit, reorder, insertion, or deletion of
///   an earlier event breaks every event downstream. [`verify`] walks the DAG
///   and reports the first break.
///
/// [`Entity`]: crate::entity::Entity
/// [`confidence`]: AuditEvent::confidence
/// [`hash`]: AuditEvent::hash
/// [`final_confidence`]: Self::final_confidence
/// [`verify`]: Self::verify
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(
        transparent,
        bound = "M::Location: Serialize + for<'a> Deserialize<'a>, \
                 M::Data: Serialize + for<'a> Deserialize<'a>"
    )
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        transparent,
        bound = "M: schemars::JsonSchema, M::Location: schemars::JsonSchema, M::Data: schemars::JsonSchema",
        rename = "{M}AuditLog"
    )
)]
pub struct AuditLog<M: Modality> {
    /// Events in the order they were recorded (a topological order of the DAG).
    events: Vec<AuditEvent<M>>,
}

/// Accessors and trail assembly that do not depend on hashing.
impl<M: Modality> AuditLog<M> {
    /// The recorded events, in order.
    pub fn events(&self) -> &[AuditEvent<M>] {
        &self.events
    }

    /// Consume the log, yielding its events in order.
    ///
    /// For a deduplication step that folds one entity's trail into another's:
    /// take the events by value and [`absorb`](Self::absorb) them, so their
    /// hashes carry over without cloning.
    pub fn into_events(self) -> Vec<AuditEvent<M>> {
        self.events
    }

    /// The number of events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether the log has no events.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// The hash at the head of the trail: the last recorded event's hash, or
    /// [`AuditHash::GENESIS`] for an empty log. This is what the next
    /// [`record`](Self::record) will link from.
    pub fn head_hash(&self) -> AuditHash {
        self.events.last().map_or(AuditHash::GENESIS, |e| e.hash)
    }

    /// Move another trail's events into this log, preserving their hashes.
    ///
    /// Used by a deduplication step before [`record_fusion`](Self::record_fusion):
    /// the absorbed events keep their own parent links (they are self-contained
    /// sub-DAGs), and the fusion event then names both trails' heads as its
    /// parents.
    pub fn absorb(&mut self, events: impl IntoIterator<Item = AuditEvent<M>>) {
        self.events.extend(events);
    }

    /// Recognition events (pattern / model) that found this entity.
    pub fn recognizers(&self) -> impl Iterator<Item = &AuditEvent<M>> {
        self.events.iter().filter(|e| e.is_recognition())
    }

    /// The entity's operator *pick*, if one was recorded: the [`Selection`]
    /// payload from its most recent selection event. `None` when no operator was
    /// picked (e.g. a suppressed entity, or before [`pick`] ran).
    ///
    /// [`Selection`]: crate::entity::audit::Selection
    /// [`pick`]: crate::entity::audit::AuditKind::Selection
    pub fn selection(&self) -> Option<&Selection> {
        self.events.iter().rev().find_map(|e| match &e.kind {
            AuditKind::Selection(s) => Some(s),
            _ => None,
        })
    }

    /// The entity's redaction, if one was recorded: the [`Redaction`] payload
    /// from its most recent redaction event. `None` when the entity was not
    /// redacted (e.g. suppressed, or not yet applied).
    ///
    /// [`Redaction`]: crate::entity::audit::Redaction
    pub fn redaction(&self) -> Option<&Redaction> {
        self.events.iter().rev().find_map(|e| match &e.kind {
            AuditKind::Redaction(r) => Some(r),
            _ => None,
        })
    }

    /// Whether an operator hid this entity — i.e. a [`Redaction`] event is on
    /// the trail.
    ///
    /// [`Redaction`]: crate::entity::audit::Redaction
    pub fn is_redacted(&self) -> bool {
        self.events
            .iter()
            .any(|e| matches!(e.kind, AuditKind::Redaction(_)))
    }

    /// The index of the first event whose [`kind`] satisfies `predicate`, or
    /// `None` if none do — the audit-trail counterpart to
    /// [`slice::position`](https://doc.rust-lang.org/std/primitive.slice.html).
    ///
    /// Events are in the order they were recorded, so comparing two positions
    /// tells you which happened first (e.g. that a [`Selection`] pick was
    /// recorded before the [`Redaction`] that applied it).
    ///
    /// [`kind`]: AuditEvent::kind
    /// [`Selection`]: crate::entity::audit::Selection
    /// [`Redaction`]: crate::entity::audit::Redaction
    pub fn position(&self, mut predicate: impl FnMut(&AuditKind<M>) -> bool) -> Option<usize> {
        self.events.iter().position(|e| predicate(&e.kind))
    }

    /// Confidence at the very first event, before any adjustment.
    pub fn original_confidence(&self) -> Option<Confidence> {
        self.events.first().map(|e| e.confidence)
    }

    /// Confidence after the most recent event: the entity's effective
    /// confidence.
    pub fn final_confidence(&self) -> Option<Confidence> {
        self.events.last().map(|e| e.confidence)
    }
}

/// Hashing and verification. Bound-free: an event's hash folds its payload and
/// its [`kind`](AuditEvent::kind) detail through [`ModalityLocation::hash`], so
/// the tamper-evident chain works with or without the `serde` feature. serde is
/// only for persisting the trail, never for building or verifying it.
///
/// [`ModalityLocation::hash`]: crate::modality::ModalityLocation::hash
impl<M: Modality> AuditLog<M> {
    /// An audit log seeded with a single birth `event` (no parents).
    pub fn new(event: AuditEvent<M>) -> Self {
        let mut log = Self { events: Vec::new() };
        log.record(event);
        log
    }

    /// Record `event` as following the current head: it links to the last
    /// recorded event (or nothing, for the first event), and its hash is
    /// computed over its payload and that parent.
    pub fn record(&mut self, mut event: AuditEvent<M>) {
        event.parents = match self.events.last() {
            Some(head) => vec![head.hash],
            None => Vec::new(),
        };
        event.hash = digest(&event);
        self.events.push(event);
    }

    /// Record a fusion `event` following several parents at once, named by
    /// their hashes: the heads of the trails being combined.
    ///
    /// The parents' own events must already have been [absorbed](Self::absorb)
    /// into this log; their hashes are unchanged, so no earlier event is
    /// re-linked. This is what a deduplication step records to join two trails
    /// without imposing a false linear order between them.
    pub fn record_fusion(&mut self, mut event: AuditEvent<M>, parents: impl Into<Vec<AuditHash>>) {
        event.parents = parents.into();
        event.hash = digest(&event);
        self.events.push(event);
    }

    /// Verify the DAG: the trail is intact and pinned to a single tip.
    ///
    /// Checks, in one pass, that
    ///
    /// - every event's [`hash`](AuditEvent::hash) recomputes from its payload
    ///   and parents (no event was altered);
    /// - every parent an event names is the hash of an *earlier* event (no
    ///   dangling link, no forward reference, no reorder that moves a parent
    ///   after its child); and
    /// - the DAG has exactly **one sink** and it is the last event, while every
    ///   other event is some later event's parent. This anchors the trail's
    ///   length and its tip: an appended tail event, an inserted orphan
    ///   (birth) event, or a deleted leaf leaves either a second sink or an
    ///   unreferenced event, and is caught here.
    ///
    /// Independent sibling events (two recognizers that later fuse) have no
    /// ordering between them, so a permutation that keeps every parent before
    /// its children is an equivalent DAG, not a tamper.
    ///
    /// Returns `Ok(())` for an intact DAG (an empty log trivially passes), or an
    /// [`Error`] of kind [`ErrorKind::Integrity`] naming the first break. This
    /// is the tamper check to run on a loaded trail.
    pub fn verify(&self) -> Result<()> {
        let mut seen: Vec<AuditHash> = Vec::with_capacity(self.events.len());
        // Which earlier events are named as a parent by some later event; the
        // events NOT in here at the end are the sinks.
        let mut referenced: Vec<AuditHash> = Vec::with_capacity(self.events.len());

        for (index, event) in self.events.iter().enumerate() {
            for parent in &event.parents {
                if !seen.contains(parent) {
                    return Err(Error::new(
                        ErrorKind::Integrity,
                        format!("audit event at index {index} names an unknown parent"),
                    ));
                }
                if !referenced.contains(parent) {
                    referenced.push(*parent);
                }
            }
            if digest(event) != event.hash {
                return Err(Error::new(
                    ErrorKind::Integrity,
                    format!("audit event at index {index} was altered: hash mismatch"),
                ));
            }
            // Each event is a distinct DAG node: a repeated hash would let a
            // duplicated event stand in as another's referenced parent, masking
            // an insertion. A genuine trail never repeats one (timestamps
            // differ), so a duplicate is a tamper.
            if seen.contains(&event.hash) {
                return Err(Error::new(
                    ErrorKind::Integrity,
                    format!("audit event at index {index} is a duplicate"),
                ));
            }
            seen.push(event.hash);
        }

        // Exactly one sink, and it is the last event: every event except the
        // last must be referenced as a parent.
        if let Some((last, rest)) = self.events.split_last() {
            for (index, event) in rest.iter().enumerate() {
                if !referenced.contains(&event.hash) {
                    return Err(Error::new(
                        ErrorKind::Integrity,
                        format!(
                            "audit event at index {index} is unreachable (trail forked or truncated)"
                        ),
                    ));
                }
            }
            if referenced.contains(&last.hash) {
                return Err(Error::new(
                    ErrorKind::Integrity,
                    "audit trail has no single tip (an event was appended past its end)".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

impl<M: Modality> Default for AuditLog<M> {
    fn default() -> Self {
        Self { events: Vec::new() }
    }
}

/// The hash of an event: BLAKE3 over its parents' hashes, its spine (source,
/// timestamp, confidence), and its [`kind`](AuditEvent::kind) detail.
///
/// The kind folds itself in through [`AuditKind::hash`], which hashes each
/// field directly (locations via [`ModalityLocation::hash`]), so no
/// serialization and no `serde` bound is involved. Parents are folded in first,
/// so an event's hash covers the exact sub-DAG it descends from.
///
/// [`ModalityLocation::hash`]: crate::modality::ModalityLocation::hash
fn digest<M: Modality>(event: &AuditEvent<M>) -> AuditHash {
    let mut bytes = Vec::new();
    for parent in &event.parents {
        bytes.extend_from_slice(parent.as_bytes());
    }
    bytes.extend_from_slice(&(event.parents.len() as u64).to_le_bytes());
    bytes.extend_from_slice(event.source.as_bytes());
    bytes.extend_from_slice(&event.timestamp.as_nanosecond().to_le_bytes());
    bytes.extend_from_slice(&event.confidence.get().to_le_bytes());
    event.kind.hash(&mut bytes);
    AuditHash::of(&bytes)
}
