//! The [`EntityBuilder`] for assembling an [`Entity`] field by field.

use uuid::Uuid;

use super::{Entity, EntityCoRef, LabelRef};
use crate::modality::Modality;
use crate::primitive::Confidence;
use crate::provenance::{Event, Provenance};

/// A chainable builder for [`Entity`].
///
/// More ergonomic than [`Entity::new`] when a producer assembles an
/// entity from a recognition event: chain
/// [`with_label`](EntityBuilder::with_label),
/// [`with_location`](EntityBuilder::with_location),
/// [`with_confidence`](EntityBuilder::with_confidence), and one or more
/// [`with_event`](EntityBuilder::with_event)s, then
/// [`build`](EntityBuilder::build). The id defaults to a fresh
/// time-ordered UUIDv7 and `coref` to unset.
///
/// ```
/// # use veil_core::entity::{Entity, EntityBuilder, LabelRef};
/// # use veil_core::modality::text::{Text, TextLocation};
/// # use veil_core::primitive::Confidence;
/// # use veil_core::provenance::{Event, PatternEvent};
/// let location = TextLocation::new(0, 11);
/// let confidence = Confidence::clamped(0.8);
/// let entity: Entity<Text> = EntityBuilder::new()
///     .with_label(LabelRef::new("US_SSN"))
///     .with_location(location.clone())
///     .with_confidence(confidence)
///     .with_event(Event::pattern("pattern", confidence, location, PatternEvent::default()))
///     .build()
///     .unwrap();
/// ```
#[derive(Debug)]
pub struct EntityBuilder<M: Modality> {
    id: Option<Uuid>,
    label: Option<LabelRef>,
    location: Option<M::Location>,
    confidence: Option<Confidence>,
    coref: Option<EntityCoRef>,
    events: Vec<Event<M>>,
}

impl<M: Modality> EntityBuilder<M> {
    /// A fresh, empty builder.
    pub fn new() -> Self {
        Self {
            id: None,
            label: None,
            location: None,
            confidence: None,
            coref: None,
            events: Vec::new(),
        }
    }

    /// Set the entity id (defaults to a fresh UUIDv7 if unset).
    #[must_use]
    pub fn with_id(mut self, id: Uuid) -> Self {
        self.id = Some(id);
        self
    }

    /// Set the label.
    #[must_use]
    pub fn with_label(mut self, label: LabelRef) -> Self {
        self.label = Some(label);
        self
    }

    /// Set the location.
    #[must_use]
    pub fn with_location(mut self, location: M::Location) -> Self {
        self.location = Some(location);
        self
    }

    /// Set the confidence.
    #[must_use]
    pub fn with_confidence(mut self, confidence: Confidence) -> Self {
        self.confidence = Some(confidence);
        self
    }

    /// Set the coreference identifier.
    #[must_use]
    pub fn with_coref(mut self, coref: EntityCoRef) -> Self {
        self.coref = Some(coref);
        self
    }

    /// Append a provenance event. Events accumulate in order.
    #[must_use]
    pub fn with_event(mut self, event: Event<M>) -> Self {
        self.events.push(event);
        self
    }

    /// Append several provenance events.
    #[must_use]
    pub fn with_events(mut self, events: impl IntoIterator<Item = Event<M>>) -> Self {
        self.events.extend(events);
        self
    }

    /// Assemble the entity.
    ///
    /// Returns [`None`] when `label`, `location`, or `confidence` was
    /// not set. The id defaults to a fresh UUIDv7; provenance is built
    /// from the accumulated events (empty if none were added).
    pub fn build(self) -> Option<Entity<M>> {
        Some(Entity {
            id: self.id.unwrap_or_else(Uuid::now_v7),
            label: self.label?,
            location: self.location?,
            confidence: self.confidence?,
            coref: self.coref,
            provenance: Provenance {
                events: self.events,
            },
        })
    }
}

impl<M: Modality> Default for EntityBuilder<M> {
    fn default() -> Self {
        Self::new()
    }
}
