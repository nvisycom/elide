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
mod recognize;
mod replacement;

pub use self::data::MetadataData;
pub use self::location::MetadataLocation;
pub use self::recognize::field_entity;
pub use self::replacement::MetadataReplacement;
use super::{Modality, NoArtifact};

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
