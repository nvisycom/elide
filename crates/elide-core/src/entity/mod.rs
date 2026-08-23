//! The detected [`Entity`] and the parts it is built from.
//!
//! An [`Entity`] is the unit that flows through the toolkit: a single
//! piece of sensitive information located somewhere in a medium, the
//! product of one or more detection layers being merged together. This
//! module also defines the entity's building blocks: the [`Label`]
//! taxonomy and the [`EntityRef`] / [`EntityCoRef`] reference types.

pub mod audit;
mod builder;
mod label;
mod reference;

use std::ops::Range;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use self::audit::{AuditEvent, AuditLog};
pub use self::builder::EntityBuilder;
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
    /// OCR layout text, the audio transcript, or the text payload itself) —
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
    /// event if any, and the redaction that hid it, as a hash-linked DAG.
    pub audit: AuditLog<M>,
    /// Whether a reviewer has suppressed this entity: it stays in the report
    /// (and keeps its audit trail, including the [`Manual`] event that
    /// suppressed it) but the redaction pass skips it, so it is never hidden.
    /// `false` for a normally-detected entity. A suppressed entity records
    /// *why* it was left alone, rather than vanishing silently.
    ///
    /// Not publicly writable: set it through [`suppress`], which also records
    /// the required [`Manual`] audit event, and read it through
    /// [`is_suppressed`]. A bare public field would let a caller flip redaction
    /// behavior with no trace on the trail. serde still deserializes it.
    ///
    /// [`Manual`]: crate::entity::audit::AuditKind::Manual
    /// [`suppress`]: Entity::suppress
    /// [`is_suppressed`]: Entity::is_suppressed
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "is_false"))]
    pub(crate) suppressed: bool,
}

/// serde `skip_serializing_if` helper: omit `suppressed` when it is the default
/// (`false`), so a normally-detected entity's wire form is unchanged.
#[cfg(feature = "serde")]
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

impl<M: Modality> Entity<M> {
    /// Assemble an entity from its location, confidence, and audit trail.
    ///
    /// Mints a fresh time-ordered [`id`] and leaves [`coref`] unset. Called
    /// by a recognizer (with a single-detection trail) or by the fusion step
    /// in `elide` (with a fused, multi-detection trail).
    ///
    /// [`id`]: Entity::id
    /// [`coref`]: Entity::coref
    pub fn new(
        label: LabelRef,
        location: M::Location,
        confidence: Confidence,
        audit: AuditLog<M>,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            label,
            location,
            confidence,
            coref: None,
            language: None,
            recognized_range: None,
            audit,
            suppressed: false,
        }
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
    /// `event` — a [`Manual`] event built with [`AuditEvent::manual`] (plus
    /// [`with_reason`]/[`with_actor`]) — is recorded onto the audit so *why* it
    /// was left alone is auditable, rather than the entity vanishing silently.
    ///
    /// [`Manual`]: crate::entity::audit::AuditKind::Manual
    /// [`AuditEvent::manual`]: crate::entity::audit::AuditEvent::manual
    /// [`with_reason`]: crate::entity::audit::AuditEvent::with_reason
    /// [`with_actor`]: crate::entity::audit::AuditEvent::with_actor
    pub fn suppress(&mut self, event: AuditEvent<M>) {
        self.suppressed = true;
        self.audit.record(event);
    }

    /// Whether a reviewer has [`suppress`](Self::suppress)ed this entity.
    #[must_use]
    pub fn is_suppressed(&self) -> bool {
        self.suppressed
    }

    /// Whether an operator has hidden this entity — a [`Redaction`] event is on
    /// its [`audit`](Self::audit) trail. A convenience for
    /// [`audit().is_redacted()`](crate::entity::audit::AuditLog::is_redacted).
    ///
    /// [`Redaction`]: crate::entity::audit::AuditKind::Redaction
    #[must_use]
    pub fn is_redacted(&self) -> bool {
        self.audit.is_redacted()
    }
}
