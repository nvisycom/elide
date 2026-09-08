//! [`Metadata`] modality: a document's named out-of-band fields, addressed by key.
//!
//! Formats carry metadata separate from their content: a photo's EXIF tags, a
//! file's inode timestamps and provenance attributes, a Word document's core
//! properties. Each such field is a `key -> value` pair that can name a person,
//! place, device, or time just as content can, so a field is a redaction subject
//! in its own right.
//!
//! All metadata fields share one shape — an atomic key ([`MetadataLocation`]), a
//! text value ([`MetadataData`]), and a drop-or-rewrite treatment
//! ([`MetadataReplacement`]) — so they are one modality, [`Metadata`]. The
//! *source* of a field (an image's EXIF, a file's inode, a document's
//! properties) is not a modality distinction: an artifact exposes metadata as one
//! of its aspects, and the engine tells two artifacts' metadata apart by their
//! `PartId`, not by the modality type. Each format's handler reads and writes its
//! own metadata within its own single encode.

mod data;
mod location;
mod replacement;

pub use self::data::MetadataData;
pub use self::location::MetadataLocation;
pub use self::replacement::MetadataReplacement;
use super::{Modality, NoArtifact};
use crate::entity::audit::{AuditEvent, MetadataEvent};
use crate::entity::{Entity, LabelRef};
use crate::primitive::Confidence;

/// A document's out-of-band named fields — EXIF tags, filesystem timestamps and
/// attributes, document properties — as redaction subjects.
///
/// One modality for every metadata source: a field is a keyed
/// [`MetadataLocation`] holding [`MetadataData`], hidden by a
/// [`MetadataReplacement`]. Which artifact a field belongs to is a matter of the
/// entity's `PartId`, not the modality.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Metadata;

impl Modality for Metadata {
    type Artifact = NoArtifact;
    type Data = MetadataData;
    type Location = MetadataLocation;
    type Replacement = MetadataReplacement;

    const NAME: &'static str = "metadata";
}

impl Metadata {
    /// Build the detection entity for a metadata field.
    ///
    /// Every metadata owner (EXIF, document properties, …) recognizes a field the
    /// same mechanical way — a present field with a known `label` becomes a
    /// max-confidence entity carrying one detection event attributed to `source`
    /// (the reader that surfaced it) — and differs only in its key→label mapping.
    /// This is that shared mechanical half, so each owner writes just the mapping.
    ///
    /// A metadata field is present or it is not, nothing probabilistic to weigh,
    /// so the entity carries [`Confidence::MAX`]. Returns `None` only if the
    /// entity builder rejects it, which cannot happen here: both the label and
    /// the location are always set.
    #[must_use]
    pub fn field_entity(key: &str, label: LabelRef, source: &str) -> Option<Entity<Self>> {
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
}
