//! Shared helper for turning a metadata field into an [`Entity`].
//!
//! Every metadata owner (EXIF, filesystem, …) recognizes fields the same
//! mechanical way — a present field with a known label becomes a max-confidence
//! entity carrying a detection event — and differs only in its key→label
//! mapping. This factors out the mechanical half so each owner writes just the
//! mapping.

use crate::entity::audit::{AuditEvent, MetadataEvent};
use crate::entity::{Entity, LabelRef};
use crate::modality::Modality;
use crate::modality::metadata::MetadataLocation;
use crate::primitive::Confidence;

/// Build the detection entity for a metadata field.
///
/// The field at `key` is labelled `label` and attributed to `source` (the
/// reader that surfaced it). A metadata field is present or it is not — there is
/// nothing probabilistic to weigh — so the entity carries [`Confidence::MAX`]
/// and a single metadata detection event.
///
/// Returns `None` only if the entity builder rejects it, which cannot happen
/// here: both the label and the location are always set.
#[must_use]
pub fn field_entity<M>(key: &str, label: LabelRef, source: &str) -> Option<Entity<M>>
where
    M: Modality<Location = MetadataLocation>,
{
    let location = MetadataLocation::new(key.to_owned());
    let event = AuditEvent::metadata(
        source,
        Confidence::MAX,
        location.clone(),
        MetadataEvent {
            source: source.to_owned().into(),
        },
    );
    Entity::builder()
        .with_label(label)
        .with_location(location)
        .with_confidence(Confidence::MAX)
        .with_event(event)
        .build()
}
