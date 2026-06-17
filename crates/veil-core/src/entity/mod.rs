//! The detected entity and its constituent parts.
//!
//! An [`Entity`] is the unit that flows through the toolkit: a single
//! piece of sensitive information located somewhere in a medium, the
//! product of one or more detection layers being merged together. This
//! module also defines the entity's building blocks — the [`Label`]
//! taxonomy and the [`EntityRef`] / [`EntityCoRef`] reference types.

mod builder;
mod label;
mod reference;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use self::builder::EntityBuilder;
pub use self::label::{builtins, Label, LabelCatalog, LabelRef};
pub use self::reference::{EntityCoRef, EntityRef};
use crate::modality::Modality;
use crate::primitive::Confidence;
use crate::provenance::Provenance;

/// A detected piece of sensitive information within some medium.
///
/// Generic over the [`Modality`] `M`, which is what makes the model
/// multimodal: a text pipeline yields `Entity<Text>`, an audio pipeline
/// `Entity<Audio>`, and so on. The entity's location is the modality's
/// [`Location`](Modality::Location) type, `M::Location`.
///
/// # Birth and fusion
///
/// A recognizer emits an entity directly, carrying a single recognition
/// [`Event`](crate::provenance::Event) (its own finding) in the entity's
/// [`provenance`](Entity::provenance). When several recognizers find the
/// same thing, a fusion step (in `veil-toolkit`) combines their entities
/// into one: the survivor's [`location`](Entity::location) and
/// [`confidence`](Entity::confidence) are the *fused* values, and every
/// contributing recognition event — plus a deduplication event — is
/// retained in its provenance. The entity therefore carries its full
/// audit trail with it.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>")
)]
pub struct Entity<M: Modality> {
    /// Stable unique identity for this entity (time-ordered UUIDv7),
    /// minted when the entity is assembled.
    pub id: Uuid,
    /// What kind of sensitive information this is (resolved via a
    /// [`LabelCatalog`]).
    pub label: LabelRef,
    /// The location of the entity within the medium (fused, if it came
    /// from more than one detection).
    pub location: M::Location,
    /// The effective confidence in `0.0..=1.0` (fused, if applicable).
    pub confidence: Confidence,
    /// Coreference identifier, if a recognizer resolved this entity as
    /// one mention of a cluster. Entities sharing an [`EntityCoRef`]
    /// denote the same real-world thing.
    pub coref: Option<EntityCoRef>,
    /// The detection audit trail: every contributing detection and the
    /// fusion event, if any.
    pub provenance: Provenance<M>,
}

impl<M: Modality> Entity<M> {
    /// Assemble an entity from its location, confidence, and provenance.
    ///
    /// Mints a fresh time-ordered [`id`](Entity::id) and leaves
    /// [`coref`](Entity::coref) unset. Called by a recognizer (with a
    /// single-detection provenance) or by the fusion step in
    /// `veil-toolkit` (with a fused, multi-detection provenance).
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
            provenance,
        }
    }

    /// Start a chainable [`EntityBuilder`].
    pub fn builder() -> EntityBuilder<M> {
        EntityBuilder::new()
    }

    /// A lightweight reference to this entity, by its [`id`](Entity::id).
    pub fn as_ref(&self) -> EntityRef {
        EntityRef::new(self.id)
    }

    /// Set the entity's coreference identifier, consuming and returning `self`.
    pub fn with_coref(mut self, coref: EntityCoRef) -> Self {
        self.coref = Some(coref);
        self
    }
}
