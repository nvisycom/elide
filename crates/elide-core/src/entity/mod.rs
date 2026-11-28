//! The detected [`Entity`]: its label, location, confidence, and audit trail.
//!
//! An [`Entity`] is the unit that flows through the toolkit: a single
//! piece of sensitive information located somewhere in a medium, the
//! product of one or more detection layers being merged together. This
//! module also defines the entity's building blocks: the [`Label`]
//! taxonomy and the [`EntityCoRef`] coreference identifier.

pub mod audit;
mod birth;
mod coref;
mod label;
mod state;

use std::collections::HashSet;
use std::ops::Range;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use self::audit::{
    Annotated, AuditEvent, AuditHash, AuditHasher, AuditLog, Fused, Manual, ManualIntent, Standing,
};
#[cfg(feature = "fixtures")]
use self::audit::{Pattern, PatternRule};
pub use self::birth::{Birth, BirthKind};
pub use self::coref::EntityCoRef;
pub use self::label::{Category, Label, LabelCatalog, LabelLocale, LabelRef, builtins};
use self::state::State;
#[cfg(feature = "fixtures")]
use crate::modality::text::{Text, TextLocation};
use crate::modality::{Modality, ModalityLocation};
use crate::primitive::{Confidence, LanguageTag};
use crate::{Error, Result};

/// Detected piece of sensitive information within some medium.
///
/// Generic over the [`Modality`] `M`, which is what makes the model
/// multimodal: a text pipeline yields `Entity<Text>`, an audio pipeline
/// `Entity<Audio>`, and so on. The entity's location is the modality's
/// [`Location`] type, `M::Location`.
///
/// # The entity is its trail
///
/// Everything an entity says is what its [`audit`] trail records, read along
/// the trail's [line](AuditLog::line) from its birth to its tip: its label,
/// location, and confidence, where it matched in the recognized text, its
/// language and coreference cluster, whether detection dropped it or a
/// reviewer suppressed it, and the operator picked and applied. The fields are that reading, kept as it
/// changes: an entity changes only by recording an event, and an entity read
/// back whose fields disagree with its trail is refused.
///
/// A recognizer emits an entity born from a single recognition
/// [`AuditEvent`] (its own finding). When several recognizers find the same
/// thing, a fusion step (in `elide-detection`) combines their entities into
/// one: the survivor's [`location`] and [`confidence`] are the *fused*
/// values, and every contributing recognition event, plus a fusion event, is
/// retained in its audit trail.
///
/// [`Location`]: Modality::Location
/// [`AuditEvent`]: crate::entity::audit::AuditEvent
/// [`audit`]: Entity::audit
/// [`location`]: Entity::location
/// [`confidence`]: Entity::confidence
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(
        bound = "M::Location: Serialize + for<'a> Deserialize<'a>",
        try_from = "EntityWire<M>"
    )
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        bound = "M: schemars::JsonSchema, M::Location: schemars::JsonSchema",
        rename = "{M}Entity"
    )
)]
pub struct Entity<M: Modality> {
    /// Stable unique identity for this entity (time-ordered UUIDv7), minted
    /// when the entity is assembled.
    pub(crate) id: Uuid,
    /// What the entity says, read from its trail.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub(crate) state: State<M>,
    /// Tamper-evident audit trail: every contributing detection, the fusion
    /// event if any, every decision and annotation, and the redaction that hid
    /// it, as a hash-linked DAG. The state is read from it.
    pub(crate) audit: AuditLog<M>,
}

/// An [`Entity`] as it arrives on the wire, before it is checked against its
/// trail.
#[cfg(feature = "serde")]
#[derive(Deserialize)]
#[serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        bound = "M: schemars::JsonSchema, M::Location: schemars::JsonSchema",
        rename = "{M}Entity"
    )
)]
struct EntityWire<M: Modality> {
    id: Uuid,
    #[serde(flatten)]
    state: State<M>,
    audit: AuditLog<M>,
}

#[cfg(feature = "serde")]
impl<M: Modality> TryFrom<EntityWire<M>> for Entity<M> {
    type Error = Error;

    /// The entity its trail records, refused when the state it arrived with
    /// says otherwise.
    fn try_from(wire: EntityWire<M>) -> Result<Self> {
        let entity = Self::from_trail(wire.id, wire.audit)?;
        if !entity.state.agrees(&wire.state) {
            return Err(Error::integrity(format!(
                "entity `{}` says what its audit trail does not",
                wire.id
            )));
        }
        Ok(entity)
    }
}

impl<M: Modality> Entity<M> {
    /// The entity `birth` starts: what it found, recorded as the first event
    /// of its trail.
    pub fn new(birth: Birth<M>) -> Self {
        let mut audit = AuditLog::default();
        audit.record(birth.event);
        Self {
            id: birth.id,
            state: birth.state,
            audit,
        }
    }

    /// The entity `id` its trail `audit` records: born from the trail's first
    /// event on its [line](AuditLog::line), with every later event on it
    /// applied in turn.
    ///
    /// # Errors
    ///
    /// [`Integrity`](crate::ErrorKind::Integrity) when the trail is empty, or
    /// its line does not start with a recognition or an inclusion.
    pub fn from_trail(id: Uuid, audit: AuditLog<M>) -> Result<Self> {
        let mut line: Vec<_> = audit.line().collect();
        line.reverse();
        let Some((birth, rest)) = line.split_first() else {
            return Err(Error::integrity(format!(
                "entity `{id}` has no audit trail"
            )));
        };
        let Some(mut state) = State::born(birth) else {
            return Err(Error::integrity(format!(
                "entity `{id}`'s trail does not start with a recognition or an inclusion"
            )));
        };
        for event in rest {
            state.apply(event);
        }
        Ok(Self { id, state, audit })
    }

    /// The entity's identity, a time-ordered UUIDv7.
    #[must_use]
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// What kind of sensitive information the entity is.
    #[must_use]
    pub fn label(&self) -> &LabelRef {
        &self.state.label
    }

    /// Where the entity sits in its medium.
    #[must_use]
    pub fn location(&self) -> &M::Location {
        &self.state.location
    }

    /// The entity's confidence, the confidence of its trail's last event.
    #[must_use]
    pub fn confidence(&self) -> Confidence {
        self.state.confidence
    }

    /// The coreference cluster the entity is one mention of, if resolved.
    #[must_use]
    pub fn coref(&self) -> Option<&EntityCoRef> {
        self.state.coref.as_ref()
    }

    /// The language of the entity's surrounding text, if resolved.
    #[must_use]
    pub fn language(&self) -> Option<&LanguageTag> {
        self.state.language.as_ref()
    }

    /// Where the entity matched in the text it was recognized in, the key back
    /// into an OCR layout or a transcript.
    #[must_use]
    pub fn recognized_range(&self) -> Option<&Range<usize>> {
        self.state.recognized_range.as_ref()
    }

    /// The entity's audit trail.
    #[must_use]
    pub fn audit(&self) -> &AuditLog<M> {
        &self.audit
    }

    /// Give the entity `location`, the location it has in another coordinate
    /// space for the same content.
    ///
    /// For moving a finding between coordinate spaces that name the same
    /// content: from a chunk into its document, or a mark to the span of text
    /// it covers. It changes where the coordinates are measured from, not
    /// what the entity covers, so it is recorded as an [`Annotated`] event,
    /// by `"lift"`; a location it already has records nothing. A change of what
    /// it covers is a decision, recorded by
    /// [`record_manual`](Self::record_manual) or [`fuse`](Self::fuse).
    pub fn lift(&mut self, location: M::Location) {
        if location.hash() != self.state.location.hash() {
            self.annotate("lift", Annotated::new().with_location(location));
        }
    }

    /// Record what `source` found out about the entity besides a decision, as
    /// an [`Annotated`] event, and apply it.
    pub fn annotate(&mut self, source: impl Into<HipStr<'static>>, annotation: Annotated<M>) {
        self.record(AuditEvent::new(source, self.state.confidence, annotation));
    }

    /// Fuse `absorbed`, detections of the same finding, into this entity as
    /// `fusion` decided, at the pooled `confidence`.
    ///
    /// Every absorbed trail is copied into this one, and the fusion event,
    /// recording the span covering them all, joins the trails' tips. The
    /// absorbed entities keep their own trails, so each stays a whole entity.
    /// An event the trails share is kept once, and a tip they share is joined
    /// once.
    pub fn fuse(&mut self, absorbed: &[Entity<M>], fusion: Fused<M>, confidence: Confidence) {
        // Trails that share events (two findings of one recognition) merge
        // each shared event once, and join each shared tip once.
        let mut seen: HashSet<AuditHash> =
            self.audit.events().iter().map(AuditEvent::hash).collect();
        // This entity's own tip comes first, so its line runs through its own
        // history, not an absorbed one's.
        let mut heads = Vec::with_capacity(absorbed.len() + 1);
        heads.push(self.audit.tip());
        for other in absorbed {
            if !heads.contains(&other.audit.tip()) {
                heads.push(other.audit.tip());
            }
            let events = other.audit.events().iter().cloned();
            self.audit
                .absorb(events.filter(|event| seen.insert(event.hash())));
        }
        let event = AuditEvent::new(fusion.strategy.name.clone(), confidence, fusion);
        self.state.apply(&event);
        self.audit.record_fusion(event, heads);
    }

    /// A digest of everything the entity says: its id and its trail's
    /// [tip](AuditLog::tip).
    ///
    /// Every other field is read from the trail, and the tip's hash covers
    /// every event on it, so the two cover everything. The order events are
    /// stored in is not covered, as nothing is read by it.
    pub fn digest(&self) -> AuditHash {
        let mut hasher = AuditHasher::new();
        hasher
            .raw(self.id.as_bytes())
            .raw(self.audit.tip().as_bytes());
        hasher.finish()
    }

    /// Record `event` on the audit trail, and apply what it decided.
    ///
    /// The entity's confidence becomes the event's, and what the event
    /// records takes effect: a reviewer's [`Manual`] relabel or relocation
    /// sets the field it names, a [`Fused`] event the fused location, and an
    /// [`Annotated`] event each field it sets. Every other event records what
    /// happened to the entity without changing what it says; a recognition or
    /// an inclusion after the birth is provenance, and changes nothing.
    ///
    /// [`Manual`]: crate::entity::audit::Manual
    /// [`Fused`]: crate::entity::audit::Fused
    pub fn record(&mut self, event: AuditEvent<M>) {
        self.state.apply(&event);
        self.audit.record(event);
    }

    /// Record the language of the entity's surrounding text, as an
    /// [`Annotated`] event by whoever recorded the entity's latest event.
    #[must_use]
    pub fn with_language(self, language: LanguageTag) -> Self {
        self.annotated(Annotated::new().with_language(language))
    }

    /// Record the entity's coreference cluster, as an [`Annotated`] event by
    /// whoever recorded the entity's latest event.
    #[must_use]
    pub fn with_coref(self, coref: EntityCoRef) -> Self {
        self.annotated(Annotated::new().with_coref(coref))
    }

    /// Record `annotation` by the source of the entity's latest event.
    fn annotated(mut self, annotation: Annotated<M>) -> Self {
        let source = self
            .audit
            .events()
            .last()
            .map(|event| event.source.clone())
            .unwrap_or_default();
        self.annotate(source, annotation);
        self
    }

    /// Apply a reviewer's [`Manual`] decision and record it on the trail, with
    /// this entity's confidence, through [`record`](Self::record).
    ///
    /// The decision and its record are one step, so a field a reviewer changed
    /// is always on the trail with what it was set to: a [`Relabel`] sets the
    /// label, a [`Relocate`] moves the entity to the decision's location, and a
    /// [`Suppress`] or a [`Confirm`] leaves the fields as they are, its
    /// location naming the region the decision covered. The event's confidence
    /// is this entity's own, so a caller cannot record a stale score. `actor`
    /// names who made the decision and becomes the event's source.
    ///
    /// A verdict that repeats the entity's [standing](AuditLog::standing)
    /// records nothing, so re-applying one does not grow the trail: a
    /// [`Suppress`] of an entity already dropped or suppressed, or a
    /// [`Confirm`] of one already confirmed. A confirmation followed by a drop
    /// stands as the drop, so confirming again records. Returns whether an
    /// event was recorded.
    ///
    /// [`Manual`]: crate::entity::audit::AuditKind::Manual
    /// [`Suppress`]: audit::ManualIntent::Suppress
    /// [`Confirm`]: audit::ManualIntent::Confirm
    /// [`Relabel`]: audit::ManualIntent::Relabel
    /// [`Relocate`]: audit::ManualIntent::Relocate
    pub fn record_manual(&mut self, manual: Manual<M>, actor: &str) -> bool {
        let repeated = matches!(
            (&manual.intent, self.audit.standing()),
            (
                ManualIntent::Suppress,
                Some(Standing::Dropped | Standing::Suppressed)
            ) | (ManualIntent::Confirm, Some(Standing::Confirmed))
        );
        if repeated {
            return false;
        }
        let event = AuditEvent::manual(actor.to_owned(), self.state.confidence, manual);
        self.record(event);
        true
    }

    /// Whether redaction leaves this entity alone: detection dropped it or a
    /// reviewer suppressed it, and no reviewer confirmed it since, as its
    /// audit trail records.
    ///
    /// The audit trail is the single source of truth. Delegates to
    /// [`AuditLog::is_suppressed`](crate::entity::audit::AuditLog::is_suppressed).
    #[must_use]
    pub fn is_suppressed(&self) -> bool {
        self.audit.is_suppressed()
    }

    /// Whether detection left this entity out of its findings, a [`Dropped`]
    /// event is on its [`audit`](Self::audit) line. A convenience for
    /// [`audit().dropped()`](crate::entity::audit::AuditLog::dropped), which
    /// says why.
    ///
    /// [`Dropped`]: crate::entity::audit::Dropped
    #[must_use]
    pub fn is_dropped(&self) -> bool {
        self.audit.dropped().is_some()
    }

    /// Whether an operator has hidden this entity, a [`Redacted`] event is on
    /// its [`audit`](Self::audit) trail. A convenience for
    /// [`audit().is_redacted()`](crate::entity::audit::AuditLog::is_redacted).
    ///
    /// [`Redacted`]: crate::entity::audit::AuditKind::Redacted
    #[must_use]
    pub fn is_redacted(&self) -> bool {
        self.audit.is_redacted()
    }
}

/// Test fixtures, behind the `fixtures` feature.
#[cfg(feature = "fixtures")]
#[cfg_attr(docsrs, doc(cfg(feature = "fixtures")))]
impl Entity<Text> {
    /// A test text entity for `label` over the byte range `span`.
    ///
    /// Born from a pattern recognition at `Confidence::MAX`, the standard test
    /// fixture.
    pub fn fixture(label: &str, span: Range<usize>) -> Self {
        Self::fixture_conf(label, span, Confidence::MAX)
    }

    /// Like [`fixture`](Self::fixture), but at an explicit `confidence`, for
    /// confidence-gated selection tests.
    pub fn fixture_conf(label: &str, span: Range<usize>, confidence: Confidence) -> Self {
        let location = TextLocation::inline(span);
        let label = LabelRef::new(label.to_owned());
        let found = Pattern::new(label, location, PatternRule::default());
        Entity::new(Birth::new("test", confidence, found))
    }
}

#[cfg(test)]
mod tests {
    use std::slice;

    use super::audit::{
        Attribution, Dropped, Included, ManualIntent, Pattern, PatternRule, Redacted, RuleMatch,
        Selected,
    };
    use super::*;
    use crate::ErrorKind;
    use crate::modality::text::{Text, TextLocation};
    use crate::primitive::ComponentId;
    use crate::redaction::LeakProfile;

    fn text_entity() -> Entity<Text> {
        let loc = TextLocation::inline(0..5);
        let conf = Confidence::MAX;
        Entity::new(Birth::new(
            "t",
            conf,
            Pattern::new("NAME", loc, PatternRule::default()),
        ))
    }

    /// Two recognizers' findings fused: swapping the sibling births leaves a
    /// trail that verifies, with the same tip, the same line, and so the same
    /// entity and digest: nothing is read by the order events are stored in.
    #[test]
    fn the_order_of_sibling_events_does_not_count() {
        let loc = TextLocation::inline(0..5);
        let conf = Confidence::MAX;
        let survivor = Entity::<Text>::new(Birth::new(
            "a",
            conf,
            Pattern::new("NAME", loc.clone(), PatternRule::default()),
        ));
        let other = Entity::<Text>::new(Birth::new(
            "b",
            conf,
            Pattern::new("PHONE", loc.clone(), PatternRule::default()),
        ));
        let mut fused = survivor.clone();
        fused.fuse(
            slice::from_ref(&other),
            Fused::new(ComponentId::new("max", "1"), loc.clone()),
            conf,
        );

        let mut events = fused.audit.events().to_vec();
        events.swap(0, 1);
        let mut swapped = AuditLog::default();
        swapped.absorb(events);
        assert!(swapped.verify().is_ok());
        assert_eq!(swapped.tip(), fused.audit.tip());
        let swapped = Entity::from_trail(fused.id, swapped).unwrap();
        assert_eq!(
            *swapped.label(),
            LabelRef::new("NAME"),
            "the survivor's line"
        );
        assert_eq!(swapped.digest(), fused.digest());
    }

    /// An entity read from its trail is the entity that recorded it, every
    /// field included.
    #[test]
    fn an_entity_is_its_trail() {
        let found =
            Pattern::<Text>::new("NAME", TextLocation::inline(0..5), PatternRule::default())
                .with_recognized_range(0..5);
        let mut entity = Entity::new(Birth::new("t", Confidence::MAX, found))
            .with_coref(EntityCoRef::new("c1"))
            .with_language("en".parse().unwrap());
        entity.lift(TextLocation::inline(10..15));
        entity.record_manual(
            Manual::new(
                ManualIntent::Relabel {
                    label: "PERSON".into(),
                },
                TextLocation::inline(10..15),
            ),
            "reviewer-7",
        );

        let read = Entity::from_trail(entity.id, entity.audit.clone()).unwrap();
        assert_eq!(read.state.label, entity.state.label);
        assert_eq!(read.state.location, entity.state.location);
        assert_eq!(read.state.confidence, entity.state.confidence);
        assert_eq!(read.state.coref, entity.state.coref);
        assert_eq!(read.state.language, entity.state.language);
        assert_eq!(read.state.recognized_range, Some(0..5));
        assert_eq!(read.digest(), entity.digest());
    }

    #[test]
    fn a_valid_trail_without_a_birth_cannot_rebuild_an_entity() {
        let id = Uuid::now_v7();
        let mut trail = AuditLog::<Text>::default();
        let err = Entity::from_trail(id, trail.clone()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Integrity);

        // A standalone suppression is a valid audit event but cannot supply
        // the initial label and location needed to rebuild an entity.
        trail.record(AuditEvent::manual(
            "reviewer-7",
            Confidence::MAX,
            Manual::new(ManualIntent::Suppress, TextLocation::inline(0..5)),
        ));
        assert!(trail.verify().is_ok());
        let err = Entity::from_trail(id, trail).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Integrity);
    }

    /// An entity read back whose fields disagree with its trail is refused.
    #[cfg(feature = "serde")]
    #[test]
    fn an_entity_disagreeing_with_its_trail_is_refused() {
        let entity = text_entity();
        let mut json = serde_json::to_value(&entity).unwrap();
        assert!(serde_json::from_value::<Entity<Text>>(json.clone()).is_ok());
        json["label"] = serde_json::json!("PUBLIC");
        assert!(serde_json::from_value::<Entity<Text>>(json).is_err());
    }

    #[test]
    fn the_confidence_follows_the_last_event() {
        let loc = TextLocation::inline(0..5);
        let low = Confidence::new(0.4).unwrap();
        let high = Confidence::new(0.8).unwrap();
        let mut entity = Entity::<Text>::new(Birth::new(
            "a",
            low,
            Pattern::new("NAME", loc.clone(), PatternRule::default()),
        ));
        entity.record(AuditEvent::new(
            "b",
            high,
            Pattern::new("NAME", loc, PatternRule::default()),
        ));
        assert_eq!(entity.state.confidence, high);
        assert_eq!(entity.audit.final_confidence(), Some(high));
        assert!(entity.audit.verify().is_ok());
    }

    /// A recognition after the birth is provenance: it scores the entity but
    /// does not relabel or move it.
    #[test]
    fn a_later_recognition_does_not_relabel() {
        let mut entity = text_entity();
        let elsewhere = TextLocation::inline(20..30);
        entity.record(AuditEvent::new(
            "b",
            Confidence::MAX,
            Pattern::new("PHONE", elsewhere, PatternRule::default()),
        ));
        assert_eq!(*entity.label(), LabelRef::new("NAME"));
        assert_eq!(*entity.location(), TextLocation::inline(0..5));
        let read = Entity::from_trail(entity.id, entity.audit.clone()).unwrap();
        assert_eq!(*read.label(), LabelRef::new("NAME"));
    }

    #[test]
    fn a_manual_suppression_suppresses() {
        let mut entity = text_entity();
        let loc = entity.state.location.clone();
        assert!(
            entity.record_manual(
                Manual::new(ManualIntent::Suppress, loc)
                    .with_attribution(Attribution::freeform("false positive")),
                "reviewer"
            )
        );
        assert!(entity.is_suppressed());
    }

    /// A detection drop suppresses the entity until a reviewer confirms it;
    /// the drop stays on the trail as why detection left it out.
    #[test]
    fn a_drop_suppresses_until_a_reviewer_confirms() {
        let mut entity = text_entity();
        let loc = entity.state.location.clone();
        let by = ComponentId::new("threshold", "1");
        let confidence = entity.confidence();
        entity.record(AuditEvent::dropped(
            Dropped::rejected(by.clone()),
            confidence,
        ));
        assert!(entity.is_dropped());
        assert!(entity.is_suppressed());
        assert_eq!(entity.audit().standing(), Some(Standing::Dropped));
        assert_eq!(entity.audit().verdict(), None, "no reviewer has decided");

        assert!(
            !entity.record_manual(Manual::new(ManualIntent::Suppress, loc.clone()), "reviewer"),
            "suppressing a dropped entity changes nothing, so records nothing"
        );
        entity.record_manual(Manual::new(ManualIntent::Confirm, loc), "reviewer");
        assert!(
            !entity.is_suppressed(),
            "the confirmation overrides the drop"
        );
        assert_eq!(entity.audit().standing(), Some(Standing::Confirmed));
        assert_eq!(entity.audit().dropped(), Some(&Dropped::rejected(by)));
        assert!(entity.audit().verify().is_ok());
    }

    /// A drop recorded after a confirmation suppresses the entity again, and
    /// confirming it again takes that drop back.
    #[test]
    fn a_confirmation_after_a_later_drop_records() {
        let mut entity = text_entity();
        let loc = entity.state.location.clone();
        let confidence = entity.confidence();
        entity.record_manual(Manual::new(ManualIntent::Confirm, loc.clone()), "reviewer");
        let by = ComponentId::new("threshold", "1");
        entity.record(AuditEvent::dropped(Dropped::rejected(by), confidence));
        assert!(entity.is_suppressed());

        assert!(entity.record_manual(Manual::new(ManualIntent::Confirm, loc.clone()), "reviewer"));
        assert!(!entity.is_suppressed());
        assert!(
            !entity.record_manual(Manual::new(ManualIntent::Confirm, loc), "reviewer"),
            "a confirmation that changes nothing records nothing"
        );
    }

    /// A relabel and a relocation set the fields they name, and record what
    /// they set, without suppressing the entity.
    #[test]
    fn an_amendment_sets_its_field_and_records_it() {
        let mut entity = text_entity();
        let moved = TextLocation::inline(1..4);
        assert!(
            entity.record_manual(
                Manual::new(
                    ManualIntent::Relabel {
                        label: LabelRef::new("ORG"),
                    },
                    entity.state.location.clone(),
                )
                .with_attribution(Attribution::freeform("retagged")),
                "reviewer-7"
            )
        );
        assert!(entity.record_manual(
            Manual::new(ManualIntent::Relocate, moved.clone()),
            "reviewer-7"
        ));

        assert_eq!(entity.state.label, LabelRef::new("ORG"));
        assert_eq!(entity.state.location, moved);
        assert!(!entity.is_suppressed(), "an amendment does not suppress");
        assert!(entity.audit.verify().is_ok());
    }

    /// An inclusion after the birth is provenance, like a repeat recognition:
    /// it changes nothing but the confidence, and the trail replays to the
    /// same entity.
    #[test]
    fn a_late_inclusion_changes_nothing() {
        let mut entity = text_entity();
        let (label, location) = (entity.label().clone(), entity.location().clone());
        entity.record(AuditEvent::new(
            "reviewer-7",
            Confidence::MAX,
            Included::new("ORG", TextLocation::inline(0..3)),
        ));
        assert_eq!((entity.label(), entity.location()), (&label, &location));
        assert_eq!(entity.confidence(), Confidence::MAX);
        let read = Entity::from_trail(entity.id, entity.audit.clone()).unwrap();
        assert_eq!((read.label(), read.location()), (&label, &location));
    }

    /// `record_manual(Suppress)` is idempotent: a second call records nothing
    /// and returns false, so re-applying a suppression does not grow the trail.
    #[test]
    fn record_manual_suppress_is_idempotent() {
        let mut entity = text_entity();
        assert!(
            entity.record_manual(
                Manual::new(ManualIntent::Suppress, entity.state.location.clone()),
                "reviewer-7"
            ),
            "first suppress records",
        );
        let after_first = entity.audit.events().len();
        assert!(entity.is_suppressed());
        assert!(
            !entity.record_manual(
                Manual::new(ManualIntent::Suppress, entity.state.location.clone()),
                "reviewer"
            ),
            "second suppress records nothing",
        );
        assert_eq!(
            entity.audit.events().len(),
            after_first,
            "re-suppressing does not grow the trail",
        );
        assert!(entity.audit.verify().is_ok());
    }

    /// Fusing findings whose trails share an event keeps it once, and the
    /// fused trail still verifies.
    #[test]
    fn fusing_shared_trails_keeps_each_event_once() {
        let mut base = text_entity();
        let absorbed = [base.clone(), base.clone()];
        let conf = base.state.confidence;
        let location = base.state.location.clone();
        base.fuse(
            &absorbed,
            Fused::new(ComponentId::new("max", "1"), location),
            conf,
        );

        assert_eq!(base.audit.events().len(), 2, "one birth, one fusion");
        assert!(base.audit.verify().is_ok());
    }

    /// The entities a fusion absorbs keep their trails, so each still reads
    /// back as itself.
    #[test]
    fn fusing_leaves_the_absorbed_whole() {
        let mut base = text_entity();
        let other = text_entity();
        let conf = base.state.confidence;
        let location = base.state.location.clone();
        base.fuse(
            slice::from_ref(&other),
            Fused::new(ComponentId::new("max", "1"), location),
            conf,
        );

        assert_eq!(other.audit.events().len(), 1);
        let read = Entity::from_trail(other.id, other.audit.clone()).unwrap();
        assert_eq!(read.digest(), other.digest());
        assert!(base.audit.verify().is_ok());
    }

    /// A redaction applies a selection on its own line: one naming a selection
    /// on an absorbed trail does not verify.
    #[test]
    fn a_redaction_applies_a_selection_on_its_own_line() {
        let pick = |entity: &mut Entity<Text>| {
            let selection = Selected::new(
                ComponentId::new("mask", "1"),
                LeakProfile::Partial,
                RuleMatch::Fallback,
            );
            entity.record(AuditEvent::selected(selection, Confidence::MAX));
            entity.audit.selection_event().unwrap().hash()
        };
        let mut absorbed = text_entity();
        let theirs = pick(&mut absorbed);
        let mut base = text_entity();
        let location = base.state.location.clone();
        base.fuse(
            &[absorbed],
            Fused::new(ComponentId::new("max", "1"), location.clone()),
            Confidence::MAX,
        );
        base.record(AuditEvent::redacted(
            "mask",
            Redacted::new(theirs, location),
            Confidence::MAX,
        ));

        assert!(base.audit.verify().is_err());
        assert!(base.audit.applied_selection().is_none());
    }

    /// A decision recorded through [`Entity::record`] still takes effect: no
    /// event is on the trail without its effect on the entity.
    #[test]
    fn a_recorded_decision_takes_effect() {
        let mut entity = text_entity();
        let moved = TextLocation::inline(2..4);
        entity.record(AuditEvent::manual(
            "reviewer-7",
            entity.state.confidence,
            Manual::new(
                ManualIntent::Relabel {
                    label: LabelRef::new("ORG"),
                },
                entity.state.location.clone(),
            ),
        ));
        entity.record(AuditEvent::fused(
            ComponentId::new("max", "1"),
            entity.state.confidence,
            moved.clone(),
        ));

        assert_eq!(entity.state.label, LabelRef::new("ORG"));
        assert_eq!(entity.state.location, moved);
        assert!(entity.audit.verify().is_ok());
    }

    /// An amendment *after* a `Suppress` must not clear the suppression: it
    /// decides nothing about redaction, so the earlier `Suppress` still stands.
    #[test]
    fn an_amendment_after_a_suppression_stays_suppressed() {
        let mut entity = text_entity();
        let loc = entity.state.location.clone();
        entity.record_manual(
            Manual::new(ManualIntent::Suppress, loc.clone()),
            "reviewer-7",
        );
        assert!(entity.is_suppressed(), "suppressed by the reviewer");
        entity.record_manual(
            Manual::new(
                ManualIntent::Relabel {
                    label: LabelRef::new("ORG"),
                },
                loc,
            ),
            "reviewer-7",
        );
        assert!(
            entity.is_suppressed(),
            "an amendment after a suppression keeps it",
        );
        assert!(entity.audit.verify().is_ok());
    }
}
