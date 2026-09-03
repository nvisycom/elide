//! [`EntityBuilder`] for assembling an [`Entity`] field by field.

use std::ops::Range;

use uuid::Uuid;

use super::{Entity, EntityCoRef, LabelRef};
use crate::entity::audit::{AuditEvent, AuditLog};
use crate::modality::Modality;
use crate::primitive::{Confidence, LanguageTag};

/// Chainable builder for [`Entity`].
///
/// More ergonomic than [`Entity::new`] when a producer assembles an
/// entity from a recognition event: chain [`with_label`],
/// [`with_location`], [`with_confidence`], and one or more
/// [`with_event`]s, then [`build`]. The id defaults to a fresh
/// time-ordered UUIDv7 and `coref` to unset.
///
/// ```
/// # use elide_core::entity::{Entity, EntityBuilder, LabelRef};
/// # use elide_core::modality::text::{Text, TextLocation};
/// # use elide_core::primitive::Confidence;
/// # use elide_core::entity::audit::{AuditEvent, PatternEvent};
/// let location = TextLocation::new(0, 11);
/// let confidence = Confidence::clamped(0.8);
/// let entity: Entity<Text> = EntityBuilder::new()
///     .with_label(LabelRef::new("US_SSN"))
///     .with_location(location.clone())
///     .with_confidence(confidence)
///     .with_event(AuditEvent::pattern("pattern", confidence, location, PatternEvent::default()))
///     .build()
///     .unwrap();
/// ```
///
/// [`with_label`]: EntityBuilder::with_label
/// [`with_location`]: EntityBuilder::with_location
/// [`with_confidence`]: EntityBuilder::with_confidence
/// [`with_event`]: EntityBuilder::with_event
/// [`build`]: EntityBuilder::build
#[derive(Debug)]
pub struct EntityBuilder<M: Modality> {
    id: Option<Uuid>,
    label: Option<LabelRef>,
    location: Option<M::Location>,
    confidence: Option<Confidence>,
    coref: Option<EntityCoRef>,
    language: Option<LanguageTag>,
    recognized_range: Option<Range<usize>>,
    events: Vec<AuditEvent<M>>,
}

impl<M: Modality> EntityBuilder<M> {
    /// Fresh, empty builder.
    pub fn new() -> Self {
        Self {
            id: None,
            label: None,
            location: None,
            confidence: None,
            coref: None,
            language: None,
            recognized_range: None,
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

    /// Append an audit event. Events accumulate in order.
    #[must_use]
    pub fn with_event(mut self, event: AuditEvent<M>) -> Self {
        self.events.push(event);
        self
    }

    /// Append several audit events.
    #[must_use]
    pub fn with_events(mut self, events: impl IntoIterator<Item = AuditEvent<M>>) -> Self {
        self.events.extend(events);
        self
    }

    /// Assemble the entity.
    ///
    /// Returns [`None`] when `label` or `location` was not set, the two facts a
    /// builder cannot default. `confidence` defaults to
    /// [`MAX`](Confidence::MAX) when unset (the shape of a user-asserted entity),
    /// so a manual add need not supply one; a recognizer sets it explicitly. The
    /// id defaults to a fresh UUIDv7; the audit trail records the accumulated
    /// events in order, each linked to the one before it, and is empty when none
    /// were added (as for a manual entity the include path stamps a Manual event
    /// onto).
    pub fn build(self) -> Option<Entity<M>> {
        let mut audit = AuditLog::default();
        for event in self.events {
            audit.record(event);
        }
        Some(Entity {
            id: self.id.unwrap_or_else(Uuid::now_v7),
            label: self.label?,
            location: self.location?,
            confidence: self.confidence.unwrap_or(Confidence::MAX),
            coref: self.coref,
            language: self.language,
            recognized_range: self.recognized_range,
            audit,
        })
    }
}

impl<M: Modality> Default for EntityBuilder<M> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::LabelRef;
    use crate::modality::text::{Text, TextLocation};

    #[test]
    fn build_defaults_confidence_to_max_and_needs_no_event() {
        // A manual add: only label + location. Confidence defaults to MAX and no
        // audit event is required (the include path stamps a Manual one).
        let entity: Entity<Text> = EntityBuilder::new()
            .with_label(LabelRef::new("US_SSN"))
            .with_location(TextLocation::new(0, 9))
            .build()
            .expect("label and location set");
        assert_eq!(entity.confidence, Confidence::MAX);
        assert!(entity.audit.events().is_empty());
    }

    #[test]
    fn build_returns_none_without_label_or_location() {
        // Label and location are the two facts a builder cannot default.
        assert!(EntityBuilder::<Text>::new().build().is_none());
        assert!(
            EntityBuilder::<Text>::new()
                .with_label(LabelRef::new("X"))
                .build()
                .is_none(),
        );
    }
}
