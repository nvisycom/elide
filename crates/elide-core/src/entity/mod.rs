//! The detected [`Entity`]: its label, location, confidence, and audit trail.
//!
//! An [`Entity`] is the unit that flows through the toolkit: a single
//! piece of sensitive information located somewhere in a medium, the
//! product of one or more detection layers being merged together. This
//! module also defines the entity's building blocks: the [`Label`]
//! taxonomy and the [`EntityCoRef`] coreference identifier.

pub mod audit;
mod coref;
mod label;

use std::collections::HashSet;
use std::mem;
use std::ops::Range;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use self::audit::{
    Attribution, AuditEvent, AuditHash, AuditHasher, AuditKind, AuditLog, Manual, ManualIntent,
    ModelId, PatternRule,
};
pub use self::coref::EntityCoRef;
pub use self::label::{Category, Label, LabelCatalog, LabelLocale, LabelRef, builtins};
#[cfg(feature = "fixtures")]
use crate::modality::text::{Text, TextLocation};
use crate::modality::{Modality, ModalityLocation};
use crate::primitive::{Confidence, LanguageTag};
use crate::redaction::HiddenContentKind;

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
/// contributing recognition event, plus a fusion event, is
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
    pub(crate) id: Uuid,
    /// What kind of sensitive information this is (resolved via a
    /// [`LabelCatalog`]).
    pub(crate) label: LabelRef,
    /// Location of the entity within the medium (fused, if it came from more
    /// than one detection).
    pub(crate) location: M::Location,
    /// Effective confidence in `0.0..=1.0` (fused, if applicable).
    pub(crate) confidence: Confidence,
    /// Coreference identifier, if a recognizer resolved this entity as one
    /// mention of a cluster. Entities sharing an [`EntityCoRef`] denote the
    /// same real-world thing.
    pub(crate) coref: Option<EntityCoRef>,
    /// The language of this entity's surrounding text, when a recognizer
    /// resolved one. `None` when unknown or language-agnostic.
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub(crate) language: Option<LanguageTag>,
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
    pub(crate) recognized_range: Option<Range<usize>>,
    /// Tamper-evident audit trail: every contributing detection, the fusion
    /// event if any, and the redaction that hid it, as a hash-linked DAG. It is
    /// also the single source of truth for whether a reviewer
    /// [suppressed](Self::is_suppressed) the entity, a suppression is a
    /// [`Manual`] event on this trail, not a separate flag.
    ///
    /// [`Manual`]: crate::entity::audit::AuditKind::Manual
    pub(crate) audit: AuditLog<M>,
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

    /// An entity the `source` pattern recognizer found: `label` at
    /// `location`, matched by `rule`, at `confidence`. Its fields are its
    /// birth event's.
    pub fn pattern(
        source: impl Into<HipStr<'static>>,
        confidence: Confidence,
        label: impl Into<LabelRef>,
        location: M::Location,
        rule: PatternRule,
    ) -> Self {
        let label = label.into();
        Self::new(label.clone(), location.clone()).with_event(AuditEvent::pattern(
            source, confidence, label, location, rule,
        ))
    }

    /// An entity the `source` model recognizer found: `label` at `location`,
    /// matched by `model`, at `confidence`. Its fields are its birth event's.
    pub fn model(
        source: impl Into<HipStr<'static>>,
        confidence: Confidence,
        label: impl Into<LabelRef>,
        location: M::Location,
        model: ModelId,
    ) -> Self {
        let label = label.into();
        Self::new(label.clone(), location.clone()).with_event(AuditEvent::model(
            source, confidence, label, location, model,
        ))
    }

    /// An item of hidden content the `source` format flagged as `label` at
    /// `location`, of kind `content`. Present rather than matched, so it
    /// carries [`Confidence::MAX`]. Its fields are its birth event's.
    pub fn flagged(
        source: impl Into<HipStr<'static>>,
        label: impl Into<LabelRef>,
        location: M::Location,
        content: HiddenContentKind,
    ) -> Self {
        let label = label.into();
        Self::new(label.clone(), location.clone()).with_event(AuditEvent::flagged(
            source,
            Confidence::MAX,
            label,
            location,
            content,
        ))
    }

    /// An entity a reviewer included: `label` at `location`, with the id
    /// `id` the review chose, recorded as an [`Include`] by `actor` (or
    /// `"manual"`) for `attribution`. A reviewer's assertion is certain, so it
    /// carries [`Confidence::MAX`].
    ///
    /// [`Include`]: ManualIntent::Include
    pub fn included(
        id: Uuid,
        label: impl Into<LabelRef>,
        location: M::Location,
        attribution: Option<Attribution>,
        actor: Option<&str>,
    ) -> Self {
        let label = label.into();
        let mut entity = Self::new(label.clone(), location.clone());
        entity.id = id;
        let manual = Manual {
            intent: ManualIntent::Include { label },
            location,
            attribution,
        };
        entity.record_manual(manual, actor);
        entity
    }

    /// The entity's identity, a time-ordered UUIDv7.
    #[must_use]
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// What kind of sensitive information the entity is.
    #[must_use]
    pub fn label(&self) -> &LabelRef {
        &self.label
    }

    /// Where the entity sits in its medium.
    #[must_use]
    pub fn location(&self) -> &M::Location {
        &self.location
    }

    /// The entity's confidence, the confidence of its trail's last event.
    #[must_use]
    pub fn confidence(&self) -> Confidence {
        self.confidence
    }

    /// The coreference cluster the entity is one mention of, if resolved.
    #[must_use]
    pub fn coref(&self) -> Option<&EntityCoRef> {
        self.coref.as_ref()
    }

    /// The language of the entity's surrounding text, if resolved.
    #[must_use]
    pub fn language(&self) -> Option<&LanguageTag> {
        self.language.as_ref()
    }

    /// Where the entity matched in the text it was recognized in, the key back
    /// into an OCR layout or a transcript.
    #[must_use]
    pub fn recognized_range(&self) -> Option<Range<usize>> {
        self.recognized_range.clone()
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
    /// it covers. It records nothing, as it changes where the coordinates are
    /// measured from, not what the entity covers; a change of what it covers
    /// is a decision, recorded by [`record_manual`](Self::record_manual) or
    /// [`fuse`](Self::fuse).
    pub fn lift(&mut self, location: M::Location) {
        self.location = location;
    }

    /// Fuse `absorbed`, detections of the same finding, into this entity at
    /// `location`, the span covering them all, with the pooled `confidence`,
    /// as the fusion `strategy` decided.
    ///
    /// Every absorbed trail moves into this one, leaving each absorbed entity
    /// with an empty trail, and a fusion event recording `location` joins the
    /// trails' tips. An event the trails share is kept once, a tip they share
    /// is joined once, and a trail with no events contributes no tip.
    pub fn fuse(
        &mut self,
        absorbed: &mut [Entity<M>],
        strategy: impl Into<HipStr<'static>>,
        confidence: Confidence,
        location: M::Location,
    ) {
        // Trails that share events (two findings of one recognition) merge
        // each shared event once, and join each shared tip once.
        let mut seen: HashSet<AuditHash> =
            self.audit.events().iter().map(AuditEvent::hash).collect();
        let mut heads = Vec::with_capacity(absorbed.len() + 1);
        if !self.audit.is_empty() {
            heads.push(self.audit.tip());
        }
        for other in absorbed {
            if !other.audit.is_empty() && !heads.contains(&other.audit.tip()) {
                heads.push(other.audit.tip());
            }
            let events = mem::take(&mut other.audit).into_events();
            self.audit
                .absorb(events.into_iter().filter(|event| seen.insert(event.hash())));
        }
        self.location = location.clone();
        self.confidence = confidence;
        self.audit
            .record_fusion(AuditEvent::fusion(strategy, confidence, location), heads);
    }

    /// A digest of everything the entity says: its id, label, location,
    /// confidence, coreference, language, and recognized range, and every
    /// event of its audit trail by hash, in the order recorded.
    ///
    /// The order counts: a trail's readers ([`is_suppressed`](Self::is_suppressed),
    /// its latest selection and redaction) take the last event of a kind, and a
    /// trail with two sibling events swapped still verifies, with the same
    /// tip.
    ///
    /// A trail [verifies](AuditLog::verify) on its own, but the fields beside
    /// it are free to change: a label or location edited without an event
    /// leaves the trail intact. The digest covers both, so comparing it
    /// against one recorded earlier catches either.
    pub fn digest(&self) -> AuditHash {
        let mut hasher = AuditHasher::new();
        hasher
            .raw(self.id.as_bytes())
            .bytes(self.label.as_str().as_bytes())
            .bytes(&self.location.hash())
            .raw(&self.confidence.get().to_le_bytes())
            .opt(self.coref.as_ref().map(|coref| coref.as_str().as_bytes()))
            .opt(self.language.as_ref().map(|tag| tag.as_str().as_bytes()));
        match &self.recognized_range {
            Some(range) => hasher
                .byte(1)
                .raw(&(range.start as u64).to_le_bytes())
                .raw(&(range.end as u64).to_le_bytes()),
            None => hasher.byte(0),
        };
        hasher.raw(&(self.audit.events().len() as u64).to_le_bytes());
        for event in self.audit.events() {
            hasher.raw(event.hash().as_bytes());
        }
        hasher.finish()
    }

    /// Record `event` on the audit trail, taking its confidence as the
    /// entity's.
    ///
    /// The entity's confidence always matches the trail's last event.
    #[must_use]
    pub fn with_event(mut self, event: AuditEvent<M>) -> Self {
        self.record(event);
        self
    }

    /// Record `event` on the audit trail in place, the `&mut` counterpart to
    /// [`with_event`](Self::with_event), and apply what it decided.
    ///
    /// The entity's confidence becomes the event's. A reviewer's [`Manual`]
    /// decision sets the label or location it names, and a [`Fusion`] moves
    /// the entity to the fused location, so no event is on the trail without
    /// its effect on the entity. Every other event records what happened to the
    /// entity without changing its label or location.
    ///
    /// [`Manual`]: crate::entity::audit::Manual
    /// [`Fusion`]: crate::entity::audit::Fusion
    pub fn record(&mut self, event: AuditEvent<M>) {
        match &event.kind {
            AuditKind::Manual(manual) => match &manual.intent {
                ManualIntent::Include { label } => {
                    self.label = label.clone();
                    self.location = manual.location.clone();
                }
                ManualIntent::Relabel { label } => self.label = label.clone(),
                ManualIntent::Relocate => self.location = manual.location.clone(),
                ManualIntent::Suppress => {}
            },
            AuditKind::Fusion(fusion) => self.location = fusion.location.clone(),
            _ => {}
        }
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

    /// Set the entity's coreference identifier.
    #[must_use]
    pub fn with_coref(mut self, coref: EntityCoRef) -> Self {
        self.coref = Some(coref);
        self
    }

    /// Apply a reviewer's [`Manual`] decision and record it on the trail, with
    /// this entity's confidence, through [`record`](Self::record).
    ///
    /// The decision and its record are one step, so a field a reviewer changed
    /// is always on the trail with what it was set to: an [`Include`] or a
    /// [`Relabel`] sets the label, an [`Include`] or a [`Relocate`] moves the
    /// entity to the decision's location, and a [`Suppress`] leaves the fields
    /// as they are, its location naming the region the decision covered. The
    /// event's confidence is this entity's own, so a caller cannot record a
    /// stale score. `actor` names who made the decision and becomes the event's
    /// source, `"manual"` when `None`.
    ///
    /// A [`Suppress`] is idempotent: if the entity is already
    /// [suppressed](Self::is_suppressed), nothing is recorded, so re-applying
    /// one does not grow the trail. Returns whether an event was recorded.
    ///
    /// [`Manual`]: crate::entity::audit::AuditKind::Manual
    /// [`Include`]: ManualIntent::Include
    /// [`Suppress`]: ManualIntent::Suppress
    /// [`Relabel`]: ManualIntent::Relabel
    /// [`Relocate`]: ManualIntent::Relocate
    pub fn record_manual(&mut self, manual: Manual<M>, actor: Option<&str>) -> bool {
        if manual.intent == ManualIntent::Suppress && self.is_suppressed() {
            return false;
        }
        let source = actor.unwrap_or("manual").to_owned();
        self.record(AuditEvent::manual(source, self.confidence, manual));
        true
    }

    /// Whether a reviewer has suppressed this entity, as its audit trail
    /// records.
    ///
    /// The audit trail is the single source of truth. Delegates to
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
    /// A test text entity for `label` over the byte range `loc`.
    ///
    /// Born from a pattern recognition at `Confidence::MAX`, the standard test
    /// fixture.
    pub fn fixture(label: &str, loc: (usize, usize)) -> Self {
        Self::fixture_conf(label, loc, Confidence::MAX)
    }

    /// Like [`fixture`](Self::fixture), but at an explicit `confidence`, for
    /// confidence-gated selection tests.
    pub fn fixture_conf(label: &str, loc: (usize, usize), confidence: Confidence) -> Self {
        let location = TextLocation::inline(loc.0..loc.1);
        let label = LabelRef::new(label.to_owned());
        Entity::pattern("test", confidence, label, location, PatternRule::default())
    }
}

#[cfg(test)]
mod tests {
    use super::audit::{Attribution, PatternRule};
    use super::*;
    use crate::modality::text::{Text, TextLocation};

    fn text_entity() -> Entity<Text> {
        let loc = TextLocation::inline(0..5);
        let conf = Confidence::MAX;
        Entity::pattern("t", conf, "NAME", loc, PatternRule::default())
    }

    /// Two recognizers' findings fused: swapping the sibling births leaves a
    /// trail that verifies, with the same tip, but not the same digest.
    #[test]
    fn the_digest_covers_the_order_of_sibling_events() {
        let loc = TextLocation::inline(0..5);
        let conf = Confidence::MAX;
        let births = [
            AuditEvent::pattern("a", conf, "NAME", loc.clone(), PatternRule::default()),
            AuditEvent::pattern("b", conf, "NAME", loc.clone(), PatternRule::default()),
        ]
        .map(|birth| {
            let mut log = AuditLog::default();
            log.record(birth);
            log.into_events().remove(0)
        });
        let parents: Vec<_> = births.iter().map(AuditEvent::hash).collect();
        let fusion = AuditEvent::fusion("max", conf, loc.clone());
        // The same fusion over the same parents; only the births' order differs.
        let fused = |births: [AuditEvent<Text>; 2]| {
            let mut entity = Entity::<Text>::new("NAME", loc.clone());
            entity.audit.absorb(births);
            entity.audit.record_fusion(fusion.clone(), parents.clone());
            entity
        };
        let [a, b] = births;
        let entity = fused([a.clone(), b.clone()]);
        let swapped = fused([b, a]);

        assert!(swapped.audit.verify().is_ok());
        assert_eq!(swapped.audit.tip(), entity.audit.tip());
        let mut same = swapped;
        same.id = entity.id;
        assert_ne!(same.digest(), entity.digest());
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
                "NAME",
                loc.clone(),
                PatternRule::default(),
            ))
            .with_event(AuditEvent::pattern(
                "b",
                high,
                "NAME",
                loc,
                PatternRule::default(),
            ));
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
                    entity.location.clone(),
                )
                .with_attribution(Attribution::freeform("retagged")),
                Some("reviewer-7"),
            )
        );
        assert!(entity.record_manual(
            Manual::new(ManualIntent::Relocate, moved.clone()),
            Some("reviewer-7"),
        ));

        assert_eq!(entity.label, LabelRef::new("ORG"));
        assert_eq!(entity.location, moved);
        assert!(!entity.is_suppressed(), "an amendment does not suppress");
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

    /// Fusing findings whose trails share an event keeps it once, and the
    /// fused trail still verifies.
    #[test]
    fn fusing_shared_trails_keeps_each_event_once() {
        let mut base = text_entity();
        let mut absorbed = [base.clone(), base.clone()];
        let conf = base.confidence;
        let location = base.location.clone();
        base.fuse(&mut absorbed, "max", conf, location);

        assert_eq!(base.audit.events().len(), 2, "one birth, one fusion");
        assert!(base.audit.verify().is_ok());
    }

    /// A decision recorded through [`Entity::record`] still takes effect: no
    /// event is on the trail without its effect on the entity.
    #[test]
    fn a_recorded_decision_takes_effect() {
        let mut entity = text_entity();
        let moved = TextLocation::inline(2..4);
        entity.record(AuditEvent::manual(
            "reviewer-7",
            entity.confidence,
            Manual::new(
                ManualIntent::Relabel {
                    label: LabelRef::new("ORG"),
                },
                entity.location.clone(),
            ),
        ));
        entity.record(AuditEvent::fusion("max", entity.confidence, moved.clone()));

        assert_eq!(entity.label, LabelRef::new("ORG"));
        assert_eq!(entity.location, moved);
        assert!(entity.audit.verify().is_ok());
    }

    /// An amendment *after* a `Suppress` must not clear the suppression: it
    /// decides nothing about redaction, so the earlier `Suppress` still stands.
    #[test]
    fn an_amendment_after_a_suppression_stays_suppressed() {
        let mut entity = text_entity();
        let loc = entity.location.clone();
        entity.record_manual(
            Manual::new(ManualIntent::Suppress, loc.clone()),
            Some("reviewer-7"),
        );
        assert!(entity.is_suppressed(), "suppressed by the reviewer");
        entity.record_manual(
            Manual::new(
                ManualIntent::Relabel {
                    label: LabelRef::new("ORG"),
                },
                loc,
            ),
            Some("reviewer-7"),
        );
        assert!(
            entity.is_suppressed(),
            "an amendment after a suppression keeps it",
        );
        assert!(entity.audit.verify().is_ok());
    }
}
