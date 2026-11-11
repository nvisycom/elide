//! The detected [`Entity`] and the parts it is built from.
//!
//! An [`Entity`] is the unit that flows through the toolkit: a single
//! piece of sensitive information located somewhere in a medium, the
//! product of one or more detection layers being merged together. This
//! module also defines the entity's building blocks: the [`Label`]
//! taxonomy and the [`EntityRef`] / [`EntityCoRef`] reference types.

pub mod audit;
mod label;
mod reference;

use std::ops::Range;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(feature = "fixtures")]
use self::audit::PatternEvent;
use self::audit::{AuditEvent, AuditLog, Manual, ManualIntent};
pub use self::label::{Category, Label, LabelCatalog, LabelLocale, LabelRef, builtins};
pub use self::reference::{EntityCoRef, EntityRef};
use crate::modality::Modality;
#[cfg(feature = "fixtures")]
use crate::modality::text::{Text, TextLocation};
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
/// `elide-detection`) combines their entities into one: the survivor's
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
    serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>")
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
    /// `label` at `location`: a fresh time-ordered [`id`], no audit events, and
    /// [`MAX`](Confidence::MAX) confidence.
    ///
    /// A detection records its birth event with [`with_event`](Self::with_event),
    /// which sets the confidence from it. An entity with no event is a located
    /// label with no provenance, such as a ciphertext span to deanonymize.
    ///
    /// [`id`]: Entity::id
    pub fn new(label: impl Into<LabelRef>, location: M::Location) -> Self {
        Self {
            id: Uuid::now_v7(),
            label: label.into(),
            location,
            confidence: Confidence::MAX,
            coref: None,
            language: None,
            recognized_range: None,
            audit: AuditLog::default(),
        }
    }

    /// Record `event` on the audit trail; the entity's confidence becomes the
    /// event's, so it always matches the trail's last event.
    #[must_use]
    pub fn with_event(mut self, event: AuditEvent<M>) -> Self {
        self.record(event);
        self
    }

    /// Record `event` on the audit trail in place, the `&mut` counterpart to
    /// [`with_event`](Self::with_event): the entity's confidence becomes the
    /// event's.
    pub fn record(&mut self, event: AuditEvent<M>) {
        self.confidence = event.confidence;
        self.audit.record(event);
    }

    /// Set the language of the entity's surrounding text.
    #[must_use]
    pub fn with_language(mut self, language: LanguageTag) -> Self {
        self.language = Some(language);
        self
    }

    /// Set the byte range of the match in the recognized text, the key back
    /// into the enrichment artifact (OCR layout, transcript).
    #[must_use]
    pub fn with_recognized_range(mut self, range: Range<usize>) -> Self {
        self.recognized_range = Some(range);
        self
    }

    /// Lightweight reference to this entity, by its [`id`].
    ///
    /// [`id`]: Entity::id
    pub fn as_ref(&self) -> EntityRef {
        EntityRef::new(self.id)
    }

    /// Set the entity's coreference identifier.
    #[must_use]
    pub fn with_coref(mut self, coref: EntityCoRef) -> Self {
        self.coref = Some(coref);
        self
    }

    /// Record a reviewer's [`Manual`] override on this entity's trail.
    ///
    /// `manual` says what the reviewer decided and where it applies: this
    /// entity's own location, or the region a decision covered when it spans
    /// more than one entity. The event's confidence is this entity's own, so a
    /// caller cannot record a stale score. `actor` names who made the override
    /// and becomes the event's source, `"manual"` when `None`.
    ///
    /// A [`Suppress`] override is idempotent: if the entity is already
    /// [suppressed](Self::is_suppressed), nothing is recorded, so re-applying an
    /// override does not grow the trail. `Flag` and `Amend` always record.
    /// Returns whether an event was recorded.
    ///
    /// [`Manual`]: crate::entity::audit::AuditKind::Manual
    /// [`Suppress`]: crate::entity::audit::ManualIntent::Suppress
    pub fn record_manual(&mut self, manual: Manual<M>, actor: Option<&str>) -> bool {
        if manual.intent == ManualIntent::Suppress && self.is_suppressed() {
            return false;
        }
        let source = actor.unwrap_or("manual").to_owned();
        self.record(AuditEvent::manual(source, self.confidence, manual));
        true
    }

    /// Whether a reviewer has suppressed this entity, the
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

/// Test fixtures, behind the `fixtures` feature.
#[cfg(feature = "fixtures")]
#[cfg_attr(docsrs, doc(cfg(feature = "fixtures")))]
impl Entity<Text> {
    /// A text entity for `label` over the byte range `loc`, born from a pattern
    /// recognition at `Confidence::MAX`, the standard test fixture.
    pub fn fixture(label: &str, loc: (usize, usize)) -> Self {
        Self::fixture_conf(label, loc, Confidence::MAX)
    }

    /// Like [`fixture`](Self::fixture), but at an explicit `confidence`, for
    /// confidence-gated selection tests.
    pub fn fixture_conf(label: &str, loc: (usize, usize), confidence: Confidence) -> Self {
        let location = TextLocation::inline(loc.0..loc.1);
        let event = AuditEvent::pattern(
            "test",
            confidence,
            location.clone(),
            PatternEvent::default(),
        );
        Entity::new(LabelRef::new(label.to_owned()), location).with_event(event)
    }
}

#[cfg(test)]
mod tests {
    use super::audit::{Attribution, PatternEvent};
    use super::*;
    use crate::modality::text::{Text, TextLocation};

    fn text_entity() -> Entity<Text> {
        let loc = TextLocation::inline(0..5);
        let conf = Confidence::MAX;
        let birth = AuditEvent::pattern("t", conf, loc.clone(), PatternEvent::default());
        Entity::new(LabelRef::new("NAME"), loc).with_event(birth)
    }

    #[test]
    fn a_new_entity_is_a_located_label_with_no_provenance() {
        let entity = Entity::<Text>::new("NAME", TextLocation::inline(0..5));
        assert_eq!(entity.confidence, Confidence::MAX);
        assert!(entity.audit.events().is_empty());
    }

    #[test]
    fn the_confidence_follows_the_last_event() {
        let loc = TextLocation::inline(0..5);
        let low = Confidence::new(0.4).unwrap();
        let high = Confidence::new(0.8).unwrap();
        let entity = Entity::<Text>::new("NAME", loc.clone())
            .with_event(AuditEvent::pattern(
                "a",
                low,
                loc.clone(),
                PatternEvent::default(),
            ))
            .with_event(AuditEvent::pattern("b", high, loc, PatternEvent::default()));
        assert_eq!(entity.confidence, high);
        assert_eq!(entity.audit.final_confidence(), Some(high));
        assert!(entity.audit.verify().is_ok());
    }

    #[test]
    fn a_manual_suppression_suppresses() {
        let mut entity = text_entity();
        let loc = entity.location.clone();
        assert!(
            entity.record_manual(
                Manual::new(ManualIntent::Suppress, loc)
                    .with_attribution(Attribution::freeform("false positive")),
                None,
            )
        );
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
            entity.record_manual(
                Manual::new(ManualIntent::Suppress, entity.location.clone()),
                Some("reviewer-7"),
            ),
            "first suppress records",
        );
        let after_first = entity.audit.events().len();
        assert!(entity.is_suppressed());
        assert!(
            !entity.record_manual(
                Manual::new(ManualIntent::Suppress, entity.location.clone()),
                None,
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

    /// An `Amend` *after* a `Suppress` must not clear the suppression, `Amend`
    /// is provenance-only, so the earlier `Suppress` decision still stands.
    #[test]
    fn amend_after_suppress_stays_suppressed() {
        let mut entity = text_entity();
        let loc = entity.location.clone();
        entity.record_manual(
            Manual::new(ManualIntent::Suppress, loc.clone()),
            Some("reviewer-7"),
        );
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
}
