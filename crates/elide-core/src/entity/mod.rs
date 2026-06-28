//! The detected [`Entity`] and the parts it is built from.
//!
//! An [`Entity`] is the unit that flows through the toolkit: a single
//! piece of sensitive information located somewhere in a medium, the
//! product of one or more detection layers being merged together. This
//! module also defines the entity's building blocks: the [`Label`]
//! taxonomy and the [`EntityRef`] / [`EntityCoRef`] reference types.

mod builder;
mod label;
pub mod provenance;
mod reference;

use std::ops::Range;

#[cfg(feature = "schema")]
use schemars::JsonSchema;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use self::builder::EntityBuilder;
pub use self::label::{Label, LabelCatalog, LabelRef, builtins};
use self::provenance::Provenance;
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
/// [`Event`] (its own finding) in the entity's [`provenance`]. When
/// several recognizers find the same thing, a fusion step (in
/// `elide`) combines their entities into one: the survivor's
/// [`location`] and [`confidence`] are the *fused* values, and every
/// contributing recognition event, plus a deduplication event, is
/// retained in its provenance. The entity therefore carries its full
/// audit trail with it.
///
/// [`Location`]: Modality::Location
/// [`Event`]: crate::entity::provenance::Event
/// [`provenance`]: Entity::provenance
/// [`location`]: Entity::location
/// [`confidence`]: Entity::confidence
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>, \
                   M::Data: Serialize + for<'a> Deserialize<'a>")
)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(bound = "M::Location: schemars::JsonSchema, M::Data: schemars::JsonSchema")
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
    /// recognition (e.g. a VLM box). Provenance, not a coordinate: redaction
    /// uses [`location`]; an audit uses this with the artifact.
    ///
    /// [`location`]: Entity::location
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub recognized_range: Option<Range<usize>>,
    /// Detection audit trail: every contributing detection and the fusion
    /// event, if any.
    pub provenance: Provenance<M>,
}

impl<M: Modality> Entity<M> {
    /// Assemble an entity from its location, confidence, and provenance.
    ///
    /// Mints a fresh time-ordered [`id`] and leaves [`coref`] unset. Called
    /// by a recognizer (with a single-detection provenance) or by the fusion
    /// step in `elide` (with a fused, multi-detection provenance).
    ///
    /// [`id`]: Entity::id
    /// [`coref`]: Entity::coref
    pub fn new(
        label: LabelRef,
        location: M::Location,
        confidence: Confidence,
        provenance: Provenance<M>,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            label,
            location,
            confidence,
            coref: None,
            language: None,
            recognized_range: None,
            provenance,
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
}
