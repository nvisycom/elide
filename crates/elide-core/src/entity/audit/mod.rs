//! The per-entity [`AuditLog`]: a tamper-evident DAG of [`AuditEvent`]s.
//!
//! [`AuditLog`]: crate::entity::audit::AuditLog

mod attribution;
mod event;
mod hash;
mod rule_match;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub use self::attribution::{Attribution, CitedAttribution, FreeformAttribution};
pub use self::event::{
    AuditEvent, AuditKind, Calibration, Conflict, Contested, Deduplication, Flagged, Manual,
    ManualIntent, Model, ModelId, Pattern, PatternRule, Redaction, Refinement, Selection,
};
pub use self::hash::AuditHash;
pub use self::rule_match::RuleMatch;
use crate::modality::Modality;
use crate::primitive::Confidence;
use crate::{Error, Result};

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
        bound = "M::Location: Serialize + for<'a> Deserialize<'a>"
    )
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        transparent,
        bound = "M: schemars::JsonSchema, M::Location: schemars::JsonSchema",
        rename = "{M}AuditLog"
    )
)]
pub struct AuditLog<M: Modality> {
    /// Events in the order they were recorded (a topological order of the DAG).
    events: Vec<AuditEvent<M>>,
}

impl<M: Modality> AuditLog<M> {
    /// An audit log seeded with a single birth `event` (no parents).
    pub fn new(event: AuditEvent<M>) -> Self {
        let mut log = Self { events: Vec::new() };
        log.record(event);
        log
    }

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

    /// The entity's current operator *pick*, if one was recorded: the
    /// [`Selection`] payload from its most recent selection event. `None` when
    /// no operator was picked (e.g. a suppressed entity, or before the
    /// anonymizer's pick ran).
    ///
    /// This is the plan, which a later pick can change after a redaction; what
    /// a redaction applied is [`applied_selection`](Self::applied_selection).
    ///
    /// [`Selection`]: crate::entity::audit::Selection
    pub fn selection(&self) -> Option<&Selection> {
        self.selection_event().and_then(|e| match &e.kind {
            AuditKind::Selection(s) => Some(s),
            _ => None,
        })
    }

    /// The entity's most recent [`Selection`] event, whose
    /// [`hash`](AuditEvent::hash) a [`Redaction`] applying it names.
    ///
    /// [`Selection`]: crate::entity::audit::Selection
    /// [`Redaction`]: crate::entity::audit::Redaction
    pub fn selection_event(&self) -> Option<&AuditEvent<M>> {
        self.events
            .iter()
            .rev()
            .find(|e| matches!(e.kind, AuditKind::Selection(_)))
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

    /// The [`Selection`] the entity's most recent [`Redaction`] applied, found
    /// through the hash the redaction names: which operator ran, why, and how
    /// much its output leaks. `None` when the entity was not redacted, or when
    /// the named selection is not on this trail (which [`verify`] reports).
    ///
    /// [`Selection`]: crate::entity::audit::Selection
    /// [`Redaction`]: crate::entity::audit::Redaction
    /// [`verify`]: Self::verify
    pub fn applied_selection(&self) -> Option<&Selection> {
        let redaction = self.redaction()?;
        self.events.iter().find_map(|e| match &e.kind {
            AuditKind::Selection(s) if e.hash == redaction.selection => Some(s),
            _ => None,
        })
    }

    /// Whether an operator hid this entity, i.e. a [`Redaction`] event is on
    /// the trail.
    ///
    /// [`Redaction`]: crate::entity::audit::Redaction
    pub fn is_redacted(&self) -> bool {
        self.events
            .iter()
            .any(|e| matches!(e.kind, AuditKind::Redaction(_)))
    }

    /// Whether this entity is hidden content its format flagged, i.e. a
    /// [`Flagged`] event is on the trail.
    pub fn is_flagged(&self) -> bool {
        self.events
            .iter()
            .any(|e| matches!(e.kind, AuditKind::Flagged(_)))
    }

    /// Whether a reviewer has suppressed this entity, i.e. its most recent
    /// [`Manual`] event carries [`ManualIntent::Suppress`]. The audit trail is
    /// the single source of truth: there is no separate flag to keep in sync.
    /// A [flagged](ManualIntent::Flag) or [amended](ManualIntent::Amend) entity
    /// is not suppressed, and is redacted like any detection.
    ///
    /// [`Manual`]: crate::entity::audit::Manual
    /// [`ManualIntent::Suppress`]: crate::entity::audit::ManualIntent::Suppress
    pub fn is_suppressed(&self) -> bool {
        self.events
            .iter()
            .rev()
            .find_map(|e| match &e.kind {
                // `Amend` is provenance-only, it does not decide suppression, so
                // an amendment after a `Suppress` must not clear it. Skip it and
                // read the most recent `Flag`/`Suppress` decision.
                AuditKind::Manual(m) if m.intent != ManualIntent::Amend => {
                    Some(m.intent == ManualIntent::Suppress)
                }
                _ => None,
            })
            .unwrap_or(false)
    }

    /// The index of the first event whose [`kind`] satisfies `predicate`, or
    /// `None` if none do, the audit-trail counterpart to
    /// [`Iterator::position`].
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

    /// Confidence after the most recent event: the entity's effective
    /// confidence.
    pub fn final_confidence(&self) -> Option<Confidence> {
        self.events.last().map(|e| e.confidence)
    }

    /// Record `event` as following the current head: it links to the last
    /// recorded event (or nothing, for the first event), and its hash is
    /// computed over its payload and that parent.
    pub fn record(&mut self, mut event: AuditEvent<M>) {
        event.parents = match self.events.last() {
            Some(head) => vec![head.hash],
            None => Vec::new(),
        };
        event.hash = event.digest();
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
        event.hash = event.digest();
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
    ///   after its child);
    /// - every [`Redaction`] names an *earlier* [`Selection`] event, the
    ///   decision it applied; and
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
    /// [`Error`] of kind [`ErrorKind::Integrity`](crate::ErrorKind::Integrity) naming the first break. This
    /// is the tamper check to run on a loaded trail.
    pub fn verify(&self) -> Result<()> {
        let mut seen: Vec<AuditHash> = Vec::with_capacity(self.events.len());
        // Which earlier events are named as a parent by some later event; the
        // events NOT in here at the end are the sinks.
        let mut referenced: Vec<AuditHash> = Vec::with_capacity(self.events.len());

        for (index, event) in self.events.iter().enumerate() {
            for parent in &event.parents {
                if !seen.contains(parent) {
                    return Err(Error::integrity(format!(
                        "audit event at index {index} names an unknown parent"
                    )));
                }
                if !referenced.contains(parent) {
                    referenced.push(*parent);
                }
            }
            if event.digest() != event.hash {
                return Err(Error::integrity(format!(
                    "audit event at index {index} was altered: hash mismatch"
                )));
            }
            if let AuditKind::Redaction(redaction) = &event.kind {
                let applied = self.events[..index].iter().any(|e| {
                    e.hash == redaction.selection && matches!(e.kind, AuditKind::Selection(_))
                });
                if !applied {
                    return Err(Error::integrity(format!(
                        "audit event at index {index} is a redaction naming no earlier selection"
                    )));
                }
            }
            // Each event is a distinct DAG node: a repeated hash would let a
            // duplicated event stand in as another's referenced parent, masking
            // an insertion. A genuine trail never repeats one (timestamps
            // differ), so a duplicate is a tamper.
            if seen.contains(&event.hash) {
                return Err(Error::integrity(format!(
                    "audit event at index {index} is a duplicate"
                )));
            }
            seen.push(event.hash);
        }

        // Exactly one sink, and it is the last event: every event except the
        // last must be referenced as a parent.
        if let Some((last, rest)) = self.events.split_last() {
            for (index, event) in rest.iter().enumerate() {
                if !referenced.contains(&event.hash) {
                    return Err(Error::integrity(format!(
                        "audit event at index {index} is unreachable (trail forked or truncated)"
                    )));
                }
            }
            if referenced.contains(&last.hash) {
                return Err(Error::integrity(
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

#[cfg(test)]
mod tests {
    use jiff::Timestamp;

    use super::*;
    use crate::entity::LabelRef;
    use crate::modality::Hint;
    use crate::modality::text::{Text, TextLocation};
    use crate::primitive::ComponentId;
    use crate::redaction::{HiddenContentKind, LeakProfile};

    /// `event` stamped at the Unix epoch, so its hash is reproducible.
    fn fixed(mut event: AuditEvent<Text>) -> AuditEvent<Text> {
        event.timestamp = Timestamp::UNIX_EPOCH;
        event
    }

    /// One event of every kind, each with every optional field set, chained in
    /// a single trail.
    fn every_kind() -> AuditLog<Text> {
        let at = TextLocation::inline(3..9);
        let conf = Confidence::new(0.5).unwrap();
        let operator = ComponentId::new("mask", "1");
        let why = Attribution::cited("GDPR", "Art. 17").with_rationale("erasure");
        let events = [
            AuditEvent::pattern(
                "pattern",
                conf,
                at.clone(),
                PatternRule {
                    name: "ssn".into(),
                    regex: Some(r"\d{3}".into()),
                    validator: Some("luhn".into()),
                },
            ),
            AuditEvent::model(
                "model",
                conf,
                at.clone(),
                ModelId {
                    name: "ner".into(),
                    version: Some("2".into()),
                },
            ),
            AuditEvent::flagged("codec", conf, at.clone(), HiddenContentKind::Collaboration),
            AuditEvent::deduplication("union", conf),
            AuditEvent::conflict(Conflict::new(LabelRef::new("iban"), conf, "longest"), conf),
            AuditEvent::contested(Contested::new(LabelRef::new("iban"), conf, "review"), conf),
            AuditEvent::calibration(conf, 0.5),
            AuditEvent::refinement(
                "context",
                conf,
                Refinement::new("ssn")
                    .with_hint(Hint::new(TextLocation::inline(0..2)))
                    .with_location(TextLocation::inline(0..2)),
            ),
            AuditEvent::selection(
                Selection::new(
                    operator.clone(),
                    LeakProfile::Partial,
                    RuleMatch::Tag("pii".into()),
                )
                .with_attribution(why.clone()),
                conf,
            ),
        ];
        let mut log = AuditLog::default();
        for event in events {
            log.record(fixed(event));
        }
        let selection = log.selection_event().unwrap().hash();
        log.record(fixed(AuditEvent::redaction(
            operator.name,
            Redaction::new(selection),
            conf,
        )));
        log.record(fixed(AuditEvent::manual(
            "reviewer",
            conf,
            Manual::new(ManualIntent::Suppress, at).with_attribution(why),
        )));
        log
    }

    /// Pins every kind's hash layout: a change to what an event folds into its
    /// hash, or to a discriminant it folds in, changes these digests and must be
    /// a deliberate format change.
    #[test]
    fn every_kind_hashes_to_its_pinned_digest() {
        let log = every_kind();
        assert!(log.verify().is_ok());
        let digests: Vec<String> = log.events().iter().map(|e| e.hash().to_string()).collect();
        assert_eq!(digests, PINNED);
    }

    /// The digests of [`every_kind`]'s events, in order.
    const PINNED: [&str; 11] = [
        // pattern
        "efd6aa450cea4b05d8b578402db5265e36b8d8ceba1e92dd68970b94fc6e2ff9",
        // model
        "be2a8bc8d489f497b7e7e57aad9d95576ba406a6306d7606bc91aacba48266e3",
        // flagged
        "05d48e609b150c7180fad9163c4e96bd160e540df3f2c5f7eb9c5a57b74937f3",
        // deduplication
        "d1b7c5290a168246ebb9cb2c5f4f3c16ede35f42637c7bb58dba1329d434d88c",
        // conflict
        "d7481599219daeab60f7ba1e35723a622a86fed5160a607228841fe3408b6575",
        // contested
        "aefaa238ba40ee2e07dda0ede908400dc9831e6b504c1af9f02e02175dbb7734",
        // calibration
        "19339f127087e661d20b90dc538aabc28f52dbed5b0ccb5bddafa470adae91c2",
        // refinement
        "e2e2678477a3aec58d3ff282764714ae7ec1ed06f3db3c79122e85e5d5881dab",
        // selection
        "f1f6a95e400b1a8aadec9dfe910d7ac178f3691e1c7aeaffac4b5d8486fffd72",
        // redaction
        "b43f22e45b1964e65e7e5d310ba0e15df92a3bde426486723258b89dc43e1d00",
        // manual
        "e6f24991d01066dfc708b4de5647d2b9e5c1d577a9c2104ad222511abc804a6e",
    ];

    /// Editing any recorded payload field breaks verification.
    #[test]
    fn editing_a_payload_field_breaks_verification() {
        let mut log = every_kind();
        let AuditKind::Selection(selection) = &mut log.events[8].kind else {
            panic!("the ninth event is the selection");
        };
        selection.leak_profile = LeakProfile::Irrecoverable;
        assert!(log.verify().is_err());
    }

    /// A selection recorded after a redaction changes the plan, not what the
    /// redaction applied.
    #[test]
    fn a_later_pick_does_not_change_the_applied_selection() {
        let mut log = every_kind();
        let applied = log.applied_selection().cloned().unwrap();
        let conf = Confidence::new(0.5).unwrap();
        let replan = Selection::new(
            ComponentId::new("erase", "1"),
            LeakProfile::Irrecoverable,
            RuleMatch::Fallback,
        );
        log.record(AuditEvent::selection(replan.clone(), conf));

        assert_eq!(log.selection(), Some(&replan));
        assert_eq!(log.applied_selection(), Some(&applied));
        assert!(log.verify().is_ok());
    }

    /// A redaction must name an earlier selection event: one naming any other
    /// event fails verification, even with every hash intact.
    #[test]
    fn a_redaction_naming_no_selection_breaks_verification() {
        let conf = Confidence::new(0.5).unwrap();
        let at = TextLocation::inline(0..2);
        let mut log = AuditLog::<Text>::new(AuditEvent::pattern(
            "pattern",
            conf,
            at,
            PatternRule::default(),
        ));
        let detection = log.events()[0].hash();
        log.record(AuditEvent::redaction(
            "mask",
            Redaction::new(detection),
            conf,
        ));

        assert!(log.applied_selection().is_none());
        assert!(log.verify().is_err());
    }
}
