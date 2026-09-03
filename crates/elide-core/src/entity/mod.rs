//! The detected [`Entity`] and the parts it is built from.
//!
//! An [`Entity`] is the unit that flows through the toolkit: a single
//! piece of sensitive information located somewhere in a medium, the
//! product of one or more detection layers being merged together. This
//! module also defines the entity's building blocks: the [`Label`]
//! taxonomy and the [`EntityRef`] / [`EntityCoRef`] reference types.

pub mod audit;
mod builder;
mod custom;
mod label;
mod reference;

use std::ops::Range;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use self::audit::{AuditEvent, AuditLog};
pub use self::builder::EntityBuilder;
pub use self::custom::CustomEntity;
pub use self::label::{Category, Label, LabelCatalog, LabelLocale, LabelRef, builtins};
pub use self::reference::{EntityCoRef, EntityRef};
use crate::modality::Modality;
use crate::primitive::{Confidence, LanguageTag};

/// Detected piece of sensitive information within some medium.
///
/// Generic over the [`Modality`] `M`, which is what makes the model
/// multimodal: a text pipeline yields `Entity<Text>`, an audio pipeline
/// `Entity<Audio>`, and so on. The entity's location is the modality's
/// [`Location`] type, `M::Location`.
///
/// # Birth and fusion
///
/// A recognizer emits an entity directly, carrying a single recognition
/// [`AuditEvent`] (its own finding) in the entity's [`audit`] trail. When
/// several recognizers find the same thing, a fusion step (in
/// `elide`) combines their entities into one: the survivor's
/// [`location`] and [`confidence`] are the *fused* values, and every
/// contributing recognition event, plus a deduplication event, is
/// retained in its audit trail. The entity therefore carries its full
/// audit trail with it.
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
    serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>, \
                   M::Data: Serialize + for<'a> Deserialize<'a>")
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        bound = "M: schemars::JsonSchema, M::Location: schemars::JsonSchema, M::Data: schemars::JsonSchema",
        rename = "{M}Entity"
    )
)]
pub struct Entity<M: Modality> {
    /// Stable unique identity for this entity (time-ordered UUIDv7), minted
    /// when the entity is assembled.
    pub id: Uuid,
    /// What kind of sensitive information this is (resolved via a
    /// [`LabelCatalog`]).
    pub label: LabelRef,
    /// Location of the entity within the medium (fused, if it came from more
    /// than one detection).
    pub location: M::Location,
    /// Effective confidence in `0.0..=1.0` (fused, if applicable).
    pub confidence: Confidence,
    /// Coreference identifier, if a recognizer resolved this entity as one
    /// mention of a cluster. Entities sharing an [`EntityCoRef`] denote the
    /// same real-world thing.
    pub coref: Option<EntityCoRef>,
    /// The language of this entity's surrounding text, when a recognizer
    /// resolved one. `None` when unknown or language-agnostic.
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub language: Option<LanguageTag>,
    /// Byte range of the match in the *recognized text* it was found in (the
    /// OCR layout text, the audio transcript, or the text payload itself),
    /// the stable key back into that enrichment artifact, where the rich
    /// context lives (which OCR block, which speaker) that the geometric
    /// [`location`] cannot hold. `None` for entities not found via text
    /// recognition (e.g. a VLM box). An audit key, not a coordinate: redaction
    /// uses [`location`]; an audit uses this with the artifact.
    ///
    /// [`location`]: Entity::location
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub recognized_range: Option<Range<usize>>,
    /// Tamper-evident audit trail: every contributing detection, the fusion
    /// event if any, and the redaction that hid it, as a hash-linked DAG. It is
    /// also the single source of truth for whether a reviewer
    /// [suppressed](Self::is_suppressed) the entity, a suppression is a
    /// [`Manual`] event on this trail, not a separate flag.
    ///
    /// [`Manual`]: crate::entity::audit::AuditKind::Manual
    pub audit: AuditLog<M>,
}

impl<M: Modality> Entity<M> {
    /// Assemble an entity from its location and audit trail.
    ///
    /// Mints a fresh time-ordered [`id`] and leaves [`coref`] unset. Called by a
    /// recognizer (with a single-detection trail) or by the fusion step in
    /// `elide` (with a fused, multi-detection trail).
    ///
    /// The [`confidence`] is the audit's [`final_confidence`], the confidence of
    /// its most recent event, since `entity.confidence` always equals that by
    /// construction (it is not stored twice). `audit` therefore must carry a
    /// birth event; an empty trail yields [`MAX`](Confidence::MAX).
    ///
    /// [`id`]: Entity::id
    /// [`coref`]: Entity::coref
    /// [`confidence`]: Entity::confidence
    /// [`final_confidence`]: crate::entity::audit::AuditLog::final_confidence
    pub fn new(label: LabelRef, location: M::Location, audit: AuditLog<M>) -> Self {
        let confidence = audit.final_confidence().unwrap_or(Confidence::MAX);
        Self {
            id: Uuid::now_v7(),
            label,
            location,
            confidence,
            coref: None,
            language: None,
            recognized_range: None,
            audit,
        }
    }

    /// Begin a user-asserted ("custom") entity: one a reviewer marks between
    /// detection and redaction, not produced by a recognizer.
    ///
    /// Returns a [`CustomEntity`] builder, so the reviewer's actor
    /// ([`by`](CustomEntity::by)) and rationale ([`because`](CustomEntity::because))
    /// land on the entity's single [`Manual`] audit event, rather than being
    /// dropped or forcing a second event to attach them.
    /// [`build`](CustomEntity::build) it (or pass it where an [`Entity`] is
    /// wanted, via the [`From`] impl) to finish: a fresh time-ordered [`id`],
    /// [`confidence`] [`MAX`](Confidence::MAX) (a human assertion is certain), and
    /// the audit event so its human origin is auditable and it is never mistaken
    /// for an automatic detection.
    ///
    /// Modality-agnostic: `location` is `M::Location`, so this builds a custom
    /// text span, image box, audio span, or any custom modality's own coordinate
    /// the same way.
    ///
    /// ```
    /// # use elide_core::entity::{Entity, LabelRef};
    /// # use elide_core::modality::text::{Text, TextLocation};
    /// let custom: Entity<Text> =
    ///     Entity::custom(LabelRef::new("US_SSN"), TextLocation::new(0, 9)).build();
    /// ```
    ///
    /// [`id`]: Entity::id
    /// [`confidence`]: Entity::confidence
    /// [`Manual`]: crate::entity::audit::AuditKind::Manual
    pub fn custom(label: impl Into<LabelRef>, location: M::Location) -> CustomEntity<M> {
        CustomEntity::new(label, location)
    }

    /// Start a chainable [`EntityBuilder`].
    pub fn builder() -> EntityBuilder<M> {
        EntityBuilder::new()
    }

    /// Lightweight reference to this entity, by its [`id`].
    ///
    /// [`id`]: Entity::id
    pub fn as_ref(&self) -> EntityRef {
        EntityRef::new(self.id)
    }

    /// Set the entity's coreference identifier, consuming and returning
    /// `self`.
    pub fn with_coref(mut self, coref: EntityCoRef) -> Self {
        self.coref = Some(coref);
        self
    }

    /// Mark this entity suppressed by a reviewer: it stays in the report and
    /// keeps its trail, but the redaction pass skips it, so it is never hidden.
    /// `event` **must** be a [`Manual`] event with [`ManualIntent::Suppress`],
    /// built with [`AuditEvent::manual_suppress`], or [`AuditEvent::manual`] with
    /// a [`Manual`] payload carrying the reviewer's rationale. Recording it onto
    /// the trail *is* the suppression, [`is_suppressed`](Self::is_suppressed)
    /// reads the trail, so there is no separate flag, and *why* the entity was
    /// left alone is auditable rather than the entity vanishing silently.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if `event` is not a `Manual` event with
    /// `Suppress` intent.
    ///
    /// [`Manual`]: crate::entity::audit::AuditKind::Manual
    /// [`ManualIntent::Suppress`]: crate::entity::audit::ManualIntent::Suppress
    /// [`AuditEvent::manual_suppress`]: crate::entity::audit::AuditEvent::manual_suppress
    /// [`AuditEvent::manual`]: crate::entity::audit::AuditEvent::manual
    pub fn suppress(&mut self, event: AuditEvent<M>) {
        debug_assert!(
            matches!(
                event.kind,
                audit::AuditKind::Manual(ref m) if m.intent == audit::ManualIntent::Suppress
            ),
            "suppress requires a Manual audit event with Suppress intent",
        );
        self.audit.record(event);
    }

    /// Record a reviewer's [`Manual`] override on this entity's trail, the
    /// total, confidence-safe way to stamp any [`ManualIntent`].
    ///
    /// The [`Manual`] payload is built from this entity's own `location`, and
    /// the event's `confidence` is this entity's `confidence`, so a caller can
    /// neither target the wrong span nor record a stale score. `attribution`
    /// carries the reviewer's rationale (the *why*); `actor` names who made the
    /// override and becomes the event's source (defaulting to `"manual"` when
    /// `None`).
    ///
    /// A [`Suppress`] override is idempotent: if the entity is already
    /// [suppressed](Self::is_suppressed), nothing is recorded, re-applying an
    /// override must not grow the trail. `Flag` and `Amend` always record.
    /// Returns whether an event was recorded.
    ///
    /// [`Manual`]: crate::entity::audit::AuditKind::Manual
    /// [`ManualIntent`]: crate::entity::audit::ManualIntent
    /// [`Suppress`]: crate::entity::audit::ManualIntent::Suppress
    pub fn record_manual(
        &mut self,
        intent: audit::ManualIntent,
        attribution: Option<audit::Attribution>,
        actor: Option<&str>,
    ) -> bool {
        // Idempotent suppression: re-applying it must not stack duplicate
        // events. Flag/Amend are not decisions about suppression state, so they
        // always record.
        if intent == audit::ManualIntent::Suppress && self.is_suppressed() {
            return false;
        }
        let mut manual = audit::Manual::new(intent, self.location.clone());
        if let Some(attribution) = attribution {
            manual = manual.with_attribution(attribution);
        }
        let source = actor.unwrap_or("manual").to_owned();
        self.audit
            .record(AuditEvent::manual(source, self.confidence, manual));
        true
    }

    /// Whether a reviewer has [`suppress`](Self::suppress)ed this entity, the
    /// audit trail is the single source of truth. Delegates to
    /// [`AuditLog::is_suppressed`](crate::entity::audit::AuditLog::is_suppressed).
    #[must_use]
    pub fn is_suppressed(&self) -> bool {
        self.audit.is_suppressed()
    }

    /// Whether an operator has hidden this entity, a [`Redaction`] event is on
    /// its [`audit`](Self::audit) trail. A convenience for
    /// [`audit().is_redacted()`](crate::entity::audit::AuditLog::is_redacted).
    ///
    /// [`Redaction`]: crate::entity::audit::AuditKind::Redaction
    #[must_use]
    pub fn is_redacted(&self) -> bool {
        self.audit.is_redacted()
    }
}

/// Test fixtures, behind the `test-util` feature.
#[cfg(feature = "test-util")]
#[cfg_attr(docsrs, doc(cfg(feature = "test-util")))]
impl Entity<crate::modality::text::Text> {
    /// A text entity for `label` over the byte range `loc`, born from a pattern
    /// recognition at `Confidence::MAX`, the standard test fixture.
    pub fn fixture(label: &str, loc: (usize, usize)) -> Self {
        Self::fixture_conf(label, loc, Confidence::MAX)
    }

    /// Like [`fixture`](Self::fixture), but at an explicit `confidence`, for
    /// confidence-gated selection tests.
    pub fn fixture_conf(label: &str, loc: (usize, usize), confidence: Confidence) -> Self {
        use crate::entity::audit::PatternEvent;
        use crate::modality::text::TextLocation;

        let location = TextLocation::new(loc.0, loc.1);
        let event = AuditEvent::pattern(
            "test",
            confidence,
            location.clone(),
            PatternEvent::default(),
        );
        Entity::new(
            LabelRef::new(label.to_owned()),
            location,
            AuditLog::new(event),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::audit::{Attribution, Manual, ManualIntent, PatternEvent};
    use super::*;
    use crate::modality::text::{Text, TextLocation};

    fn text_entity() -> Entity<Text> {
        let loc = TextLocation::new(0, 5);
        let conf = Confidence::MAX;
        let birth = AuditEvent::pattern("t", conf, loc.clone(), PatternEvent::default());
        Entity::new(LabelRef::new("NAME"), loc, AuditLog::new(birth))
    }

    #[test]
    fn custom_builds_a_manual_entity_at_max_confidence() {
        let loc = TextLocation::new(3, 12);
        let entity = Entity::<Text>::custom("US_SSN", loc.clone()).build();

        assert_eq!(entity.label, LabelRef::new("US_SSN"));
        assert_eq!(entity.location, loc);
        assert_eq!(
            entity.confidence,
            Confidence::MAX,
            "a human assertion is certain"
        );
        // Its sole audit event is a Manual(Flag) — human origin, auditable.
        let events = entity.audit.events();
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0].kind, super::audit::AuditKind::Manual(m)
                if m.intent == ManualIntent::Flag),
            "custom stamps a Manual(Flag) event",
        );
        assert!(entity.audit.verify().is_ok(), "the trail verifies");
        // Not a recognizer detection: no Pattern/model birth event.
        assert!(!entity.is_suppressed());
    }

    #[test]
    fn custom_carries_actor_and_attribution_on_one_event() {
        let loc = TextLocation::new(3, 12);
        let entity = Entity::<Text>::custom("US_SSN", loc)
            .by("reviewer-7")
            .because(Attribution::freeform("gdpr-art-17"))
            .build();

        // Actor and rationale land on the single Manual(Flag) event, not a second
        // one stacked to attach them after the fact.
        let events = entity.audit.events();
        assert_eq!(events.len(), 1, "one event carries both");
        assert_eq!(events[0].source, "reviewer-7");
        let super::audit::AuditKind::Manual(manual) = &events[0].kind else {
            panic!("custom stamps a Manual event");
        };
        assert_eq!(manual.intent, ManualIntent::Flag);
        assert_eq!(
            manual.attribution,
            Some(Attribution::freeform("gdpr-art-17").into()),
        );
        assert!(entity.audit.verify().is_ok(), "the trail verifies");
    }

    #[test]
    fn suppress_records_the_manual_event() {
        let mut entity = text_entity();
        let loc = entity.location.clone();
        entity.suppress(AuditEvent::manual(
            "manual",
            entity.confidence,
            Manual::new(ManualIntent::Suppress, loc)
                .with_attribution(Attribution::freeform("false positive")),
        ));
        assert!(entity.is_suppressed());
    }

    /// An amended entity is a human override (a `Manual` event) but *not*
    /// suppressed, only `Suppress` skips redaction; `Amend` is provenance-only.
    #[test]
    fn an_amended_entity_is_not_suppressed() {
        let mut entity = text_entity();
        let loc = entity.location.clone();
        entity
            .audit
            .record(AuditEvent::manual("reviewer-7", entity.confidence, {
                Manual::new(ManualIntent::Amend, loc)
                    .with_attribution(Attribution::freeform("retagged"))
            }));
        assert!(!entity.is_suppressed(), "amend does not suppress");
        assert!(entity.audit.verify().is_ok());
    }

    /// `record_manual(Suppress)` is idempotent: a second call records nothing
    /// and returns false, so re-applying a suppression does not grow the trail.
    #[test]
    fn record_manual_suppress_is_idempotent() {
        let mut entity = text_entity();
        assert!(
            entity.record_manual(ManualIntent::Suppress, None, Some("reviewer-7")),
            "first suppress records",
        );
        let after_first = entity.audit.events().len();
        assert!(entity.is_suppressed());
        assert!(
            !entity.record_manual(ManualIntent::Suppress, None, None),
            "second suppress records nothing",
        );
        assert_eq!(
            entity.audit.events().len(),
            after_first,
            "re-suppressing does not grow the trail",
        );
        assert!(entity.audit.verify().is_ok());
    }

    /// An `Amend` *after* a `Suppress` must not clear the suppression, `Amend`
    /// is provenance-only, so the earlier `Suppress` decision still stands.
    #[test]
    fn amend_after_suppress_stays_suppressed() {
        let mut entity = text_entity();
        let loc = entity.location.clone();
        entity.suppress(AuditEvent::manual(
            "reviewer-7",
            entity.confidence,
            Manual::new(ManualIntent::Suppress, loc.clone()),
        ));
        assert!(entity.is_suppressed(), "suppressed by the reviewer");
        // A later amendment (e.g. retag) is recorded but leaves the entity
        // suppressed.
        entity.audit.record(AuditEvent::manual(
            "reviewer-7",
            entity.confidence,
            Manual::new(ManualIntent::Amend, loc),
        ));
        assert!(
            entity.is_suppressed(),
            "amend after suppress keeps the suppression",
        );
        assert!(entity.audit.verify().is_ok());
    }

    /// `suppress` requires a `Manual` event; a recognition event is rejected in
    /// debug builds so suppression cannot change redaction behaviour without the
    /// human-override event that explains it.
    #[test]
    #[should_panic(expected = "suppress requires a Manual audit event")]
    fn suppress_rejects_a_non_manual_event() {
        let mut entity = text_entity();
        let loc = entity.location.clone();
        // A recognition event, not a Manual override.
        entity.suppress(AuditEvent::pattern(
            "t",
            entity.confidence,
            loc,
            PatternEvent::default(),
        ));
    }
}
