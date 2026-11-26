//! The per-entity [`AuditLog`]: a tamper-evident DAG of [`AuditEvent`]s.
//!
//! [`AuditLog`]: crate::entity::audit::AuditLog

mod attribution;
mod event;
mod hash;
mod rule_match;

use std::iter;

#[cfg(feature = "serde")]
use serde::de::Error as _;
#[cfg(feature = "serde")]
use serde::{Deserialize, Deserializer, Serialize};

pub use self::attribution::{Attribution, CitedAttribution, FreeformAttribution};
pub use self::event::{
    Annotation, AuditEvent, AuditKind, Calibration, Conflict, Contested, Flagged, Fusion, Include,
    Manual, ManualIntent, Model, ModelId, Pattern, PatternRule, Redaction, Refinement, Selection,
};
pub use self::hash::AuditHash;
pub(crate) use self::hash::AuditHasher;
pub use self::rule_match::RuleMatch;
use crate::modality::Modality;
use crate::primitive::Confidence;
use crate::{Error, Result};

/// An [`Entity`]'s full audit trail, as a tamper-evident DAG of events.
///
/// It holds every [`AuditEvent`] in the entity's life.
///
/// Where Presidio keeps a shallow, optional, per-stage explanation that is
/// stripped by default, an `AuditLog` is always present and records the
/// entity's *entire* life: each recognizer that found it, the fusion
/// that joined them, any confidence calibration, and the redaction that hid it.
/// Nothing is collapsed: every recognizer keeps its own recognition event with
/// its location and score.
///
/// The events form a **directed acyclic graph**, not a flat list. A recognizer
/// records a birth event (no parents); each later step links to the event it
/// follows; and a [fusion](crate::entity::Entity::fuse) links to *several* parents at
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
#[cfg_attr(feature = "serde", derive(Serialize))]
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

/// A trail deserializes only once it [verifies](AuditLog::verify): a loaded
/// trail whose hashes or links do not hold (an event altered, an event cut
/// below its tip, an event appended past it) is rejected as it is read, so no
/// caller can act on one without checking it. A trail cut back to an earlier
/// tip is intact in its own right and reads back; only a tip or report
/// digest held elsewhere tells it apart.
#[cfg(feature = "serde")]
impl<'de, M> Deserialize<'de> for AuditLog<M>
where
    M: Modality,
    M::Location: Serialize + for<'a> Deserialize<'a>,
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let log = Self {
            events: Vec::deserialize(deserializer)?,
        };
        log.verify().map_err(D::Error::custom)?;
        Ok(log)
    }
}

impl<M: Modality> AuditLog<M> {
    /// The recorded events, in order.
    pub fn events(&self) -> &[AuditEvent<M>] {
        &self.events
    }

    /// The hash of the trail's last event, which every earlier event's hash
    /// is folded into, or [`GENESIS`](AuditHash::GENESIS) for an empty trail.
    pub fn tip(&self) -> AuditHash {
        self.events
            .last()
            .map_or(AuditHash::GENESIS, AuditEvent::hash)
    }

    /// The number of events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether the log has no events.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Move another trail's events into this log, preserving their hashes.
    ///
    /// Used by a fusion step before [`record_fusion`](Self::record_fusion):
    /// the absorbed events keep their own parent links (they are self-contained
    /// sub-DAGs), and the fusion event then names both trails' heads as its
    /// parents.
    pub(crate) fn absorb(&mut self, events: impl IntoIterator<Item = AuditEvent<M>>) {
        self.events.extend(events);
    }

    /// The entity's own line through the trail: its events from its tip back
    /// to its birth, following each event's first parent.
    ///
    /// A [fusion](crate::entity::Entity::fuse) names the fused entity's own
    /// tip as its first parent and the absorbed trails' tips after it, so the
    /// line passes through the entity's own history and none of theirs. What
    /// the entity is (its label, location, suppression, operator, redaction)
    /// is read along this line; the absorbed events stay on the trail as
    /// provenance. The line is fixed by the hashes, not by the order events
    /// are stored in.
    pub fn line(&self) -> impl Iterator<Item = &AuditEvent<M>> {
        self.ancestry(self.events.len().checked_sub(1))
    }

    /// The event at `index` and its first-parent ancestors, back to the
    /// birth it descends from.
    fn ancestry(&self, index: Option<usize>) -> impl Iterator<Item = &AuditEvent<M>> {
        let mut next = index;
        iter::from_fn(move || {
            let index = next?;
            let event = &self.events[index];
            // A parent is always recorded before its child.
            next = event.parents.first().and_then(|parent| {
                self.events[..index]
                    .iter()
                    .rposition(|earlier| earlier.hash == *parent)
            });
            Some(event)
        })
    }

    /// Recognition events (pattern / model) that found this entity.
    pub fn recognizers(&self) -> impl Iterator<Item = &AuditEvent<M>> {
        self.events.iter().filter(|e| e.is_recognition())
    }

    /// The entity's current operator *pick*, if one was recorded: the
    /// [`Selection`] payload from the latest selection event on its
    /// [line](Self::line). `None` when no operator was picked (e.g. a
    /// suppressed entity, or before the anonymizer's pick ran).
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
        self.line()
            .find(|e| matches!(e.kind, AuditKind::Selection(_)))
    }

    /// The entity's redaction, if one was recorded: the [`Redaction`] payload
    /// from the latest redaction event on its [line](Self::line). `None` when
    /// the entity was not redacted (e.g. suppressed, or not yet applied).
    ///
    /// [`Redaction`]: crate::entity::audit::Redaction
    pub fn redaction(&self) -> Option<&Redaction<M>> {
        self.line().find_map(|e| match &e.kind {
            AuditKind::Redaction(r) => Some(r),
            _ => None,
        })
    }

    /// The [`Selection`] that the entity's most recent [`Redaction`] applied.
    ///
    /// It is found on the entity's [line](Self::line) through the hash the
    /// redaction names: which operator ran, why, and how much its output
    /// leaks. `None` when the entity was not redacted, or when the named
    /// selection is not on the line (which [`verify`] reports).
    ///
    /// [`Selection`]: crate::entity::audit::Selection
    /// [`Redaction`]: crate::entity::audit::Redaction
    /// [`verify`]: Self::verify
    pub fn applied_selection(&self) -> Option<&Selection> {
        let redaction = self.redaction()?;
        self.line().find_map(|e| match &e.kind {
            AuditKind::Selection(s) if e.hash == redaction.selection => Some(s),
            _ => None,
        })
    }

    /// Whether an operator hid this entity, i.e. a [`Redaction`] event is on
    /// its [line](Self::line).
    ///
    /// [`Redaction`]: crate::entity::audit::Redaction
    pub fn is_redacted(&self) -> bool {
        self.redaction().is_some()
    }

    /// Whether this entity is hidden content its format flagged, i.e. a
    /// [`Flagged`] event is on its [line](Self::line).
    pub fn is_flagged(&self) -> bool {
        self.line().any(|e| matches!(e.kind, AuditKind::Flagged(_)))
    }

    /// Whether a reviewer has suppressed this entity with a [`Manual`] event.
    ///
    /// That is, the latest [verdict](Self::verdict) on its
    /// [line](Self::line) is [`ManualIntent::Suppress`]. The audit trail is
    /// the single source of truth: there is no separate flag to keep in sync.
    ///
    /// [`Manual`]: crate::entity::audit::Manual
    /// [`ManualIntent::Suppress`]: crate::entity::audit::ManualIntent::Suppress
    pub fn is_suppressed(&self) -> bool {
        self.verdict() == Some(&ManualIntent::Suppress)
    }

    /// The latest reviewer verdict on this entity's [line](Self::line): a
    /// [`Suppress`] or a [`Confirm`], or `None` if no reviewer has given
    /// one. An include, a relabel or a relocation decides nothing about
    /// redaction and is skipped.
    ///
    /// [`Suppress`]: crate::entity::audit::ManualIntent::Suppress
    /// [`Confirm`]: crate::entity::audit::ManualIntent::Confirm
    pub fn verdict(&self) -> Option<&ManualIntent> {
        self.line().find_map(|e| match &e.kind {
            AuditKind::Manual(m)
                if matches!(m.intent, ManualIntent::Suppress | ManualIntent::Confirm) =>
            {
                Some(&m.intent)
            }
            _ => None,
        })
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

    /// Record `event` as following the current head of the trail.
    ///
    /// It links to the last recorded event (or nothing, for the first event),
    /// and its hash is computed over its payload and that parent.
    pub(crate) fn record(&mut self, mut event: AuditEvent<M>) {
        event.parents = match self.events.last() {
            Some(head) => vec![head.hash],
            None => Vec::new(),
        };
        event.hash = event.digest();
        self.events.push(event);
    }

    /// Record a fusion `event` that follows several parents at once.
    ///
    /// The parents are named by their hashes: the heads of the trails being
    /// combined.
    ///
    /// The parents' own events must already have been [absorbed](Self::absorb)
    /// into this log; their hashes are unchanged, so no earlier event is
    /// re-linked. This is what a fusion step records to join two trails
    /// without imposing a false linear order between them.
    pub(crate) fn record_fusion(
        &mut self,
        mut event: AuditEvent<M>,
        parents: impl Into<Vec<AuditHash>>,
    ) {
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
    /// - every [`Redaction`] names a [`Selection`] event among its own
    ///   first-parent ancestors, the decision it applied, so a redaction never
    ///   applies a selection from a trail a fusion absorbed; and
    /// - the DAG has exactly **one sink** and it is the last event, while every
    ///   other event is some later event's parent: an appended tail event, an
    ///   inserted orphan (birth) event, or a deleted leaf below the tip leaves
    ///   either a second sink or an unreferenced event, and is caught here.
    ///
    /// Independent sibling events (two recognizers that later fuse) have no
    /// ordering between them, so a permutation that keeps every parent before
    /// its children is an equivalent DAG, not a tamper.
    ///
    /// What it cannot catch is a trail rewritten whole. The hashes are unkeyed,
    /// so whoever rewrites every event can recompute every hash; and a trail
    /// cut back to an earlier event, its tip dropped, is a shorter trail that
    /// is intact in its own right. Both need the trail's tip compared against
    /// one held where its writer cannot change it: see
    /// `Report::digest` in `elide-engine`, which anchors a whole report.
    ///
    /// A trail deserializes only once it verifies.
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
                let applied = self.ancestry(Some(index)).skip(1).any(|e| {
                    e.hash == redaction.selection && matches!(e.kind, AuditKind::Selection(_))
                });
                if !applied {
                    return Err(Error::integrity(format!(
                        "audit event at index {index} is a redaction naming no selection on its line"
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
    use crate::entity::{EntityCoRef, LabelRef};
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
            AuditEvent::new(
                "pattern",
                conf,
                Pattern::new(
                    "ssn",
                    at.clone(),
                    PatternRule {
                        name: "ssn".into(),
                        regex: Some(r"\d{3}".into()),
                        validator: Some("luhn".into()),
                    },
                )
                .with_recognized_range(3..9),
            ),
            AuditEvent::new(
                "model",
                conf,
                Model::new(
                    "ssn",
                    at.clone(),
                    ModelId {
                        name: "ner".into(),
                        version: Some("2".into()),
                    },
                )
                .with_recognized_range(3..9),
            ),
            AuditEvent::new(
                "codec",
                conf,
                Flagged::new("author", at.clone(), HiddenContentKind::Collaboration),
            ),
            AuditEvent::fusion("union", conf, at.clone()),
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
            Redaction::new(selection, at.clone()),
            conf,
        )));
        log.record(fixed(AuditEvent::new(
            "reviewer",
            conf,
            Include::new("ssn", at.clone()).because(why.clone()),
        )));
        let decisions = [
            ManualIntent::Relabel {
                label: LabelRef::new("iban"),
            },
            ManualIntent::Relocate,
            ManualIntent::Suppress,
            ManualIntent::Confirm,
        ];
        for intent in decisions {
            log.record(fixed(AuditEvent::manual(
                "reviewer",
                conf,
                Manual::new(intent, at.clone()).with_attribution(why.clone()),
            )));
        }
        log.record(fixed(AuditEvent::new(
            "lift",
            conf,
            Annotation::new()
                .with_location(TextLocation::inline(13..19))
                .with_recognized_range(3..9)
                .with_language("en".parse().unwrap())
                .with_coref(EntityCoRef::new("c1")),
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
    const PINNED: [&str; 16] = [
        "4bdcf255f11836161b03d87ca7b0e4aa752819c1db0d685f31af34a53f9fd72b", // pattern
        "12b57ac561ebb7dafa19f1a6fbd46f844a03f941bf7c68eabcb4456e6343c6c8", // model
        "17d5f2ef91525e3b9ecf952034e719bb7d80e08e66ac1024f8b55c43e7a12787", // flagged
        "8eb6ada2663418d5df28fdbbba64896918186d2a7978996202a0bb61a3939864", // fusion
        "32670d83b16832696e7cb2fc9434d678914be0dc131f5c4b46029068fb920781", // conflict
        "be8fce5b547ba26d6d00708065531804cf6d532704bcb9e574f9aacecfb0edff", // contested
        "0e6863731c263b2c3055aaa397d92a411c0a9bf50b00cefc289fb25b86b598c7", // calibration
        "c6c28ab2a4c3ab9a9f5aa14e6b564f5038b3d537d77ad8444593c1f5b812caae", // refinement
        "b9de18ca37492a4198e3d1bafd680cfcd897d2ccc35f4be4738081ad76c8d8d3", // selection
        "1dc0e97acc0f1b153b13bfb9020a08de47c7a776c9e7c5b7500c5dc3a7309c41", // redaction
        "526b28ea57a81cd85890ce2aee071d84d302cd530c2164289ddc0044f2681eb1", // include
        "e6b532659e85596add3c31ea43fa44143695fc4d647b650d3bd7de78ed046cbb", // relabel
        "e6862a908b9631804d3e7f5ef8c3a0baab971ed1eb5269b3ee3f74db7ad20d3b", // relocate
        "4557866b6640890495e35d31579c3fe7b036db68532cde8a89183d865ff02612", // suppress
        "db893bbe18a1a0393885b992db47001107bf6f60bdaa9f381cd5d7837c38059d", // confirm
        "91f2510110ac32be5f6689a0a7e3644beda54b2298c9c97a7aad9dd6cd777753", // annotation
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

    /// An intact trail reads back as written; one altered in transit, or with
    /// an event cut from its middle, does not read back at all.
    #[cfg(feature = "serde")]
    #[test]
    fn a_trail_deserializes_only_once_it_verifies() {
        let log = every_kind();
        let json = serde_json::to_value(&log).unwrap();
        let read: AuditLog<Text> = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(read.events().len(), log.events().len());

        let mut altered = json.clone();
        altered[0]["confidence"] = serde_json::json!(0.99);
        assert!(serde_json::from_value::<AuditLog<Text>>(altered).is_err());

        let mut cut = json;
        cut.as_array_mut().unwrap().remove(0);
        assert!(serde_json::from_value::<AuditLog<Text>>(cut).is_err());
    }

    /// A selection recorded after a redaction changes the plan, not what the
    /// redaction applied.
    #[test]
    fn a_later_pick_does_not_change_the_applied_selection() {
        let mut log = every_kind();
        let applied = log.applied_selection().cloned().unwrap();
        let conf = Confidence::new(0.5).unwrap();
        let replan = Selection::new(
            ComponentId::new("remove", "1"),
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
        let mut log = AuditLog::<Text>::default();
        log.record(AuditEvent::new(
            "pattern",
            conf,
            Pattern::new("ssn", at.clone(), PatternRule::default()),
        ));
        let detection = log.events()[0].hash();
        log.record(AuditEvent::redaction(
            "mask",
            Redaction::new(detection, at),
            conf,
        ));

        assert!(log.applied_selection().is_none());
        assert!(log.verify().is_err());
    }
}
