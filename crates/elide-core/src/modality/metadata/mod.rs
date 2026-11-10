//! [`Metadata`] modality: a document's named out-of-band fields, addressed by key.
//!
//! Formats carry metadata separate from their content: a photo's EXIF tags, a
//! file's inode timestamps and provenance attributes, a Word document's core
//! properties. Each such field is a `key -> value` pair that can name a person,
//! place, device, or time just as content can, so a field is a redaction subject
//! in its own right.
//!
//! All metadata fields share one shape, a keyed text value ([`MetadataData`]),
//! so they are one modality, [`Metadata`]. A value is text and is recognized
//! like text: the location is the field's key, optionally narrowed to a byte
//! range of its value ([`MetadataLocation`]), and redaction is a text treatment
//! of the value ([`MetadataReplacement`]). The *source* of a field (an image's EXIF, a file's inode, a
//! document's properties) is not a modality distinction: an artifact exposes
//! metadata as one of its aspects, and the engine tells two artifacts' metadata
//! apart by their `PartId`, not by the modality type. Each format's handler
//! reads and writes its own metadata within its own single encode.

mod data;
mod location;
mod replacement;

use std::ops::Range;

pub use self::data::MetadataData;
pub use self::location::MetadataLocation;
pub use self::replacement::MetadataReplacement;
use super::{Modality, NoArtifact, TextRecognizable};

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

impl TextRecognizable for Metadata {
    fn as_text<'a>(data: &'a MetadataData, _artifact: Option<&'a NoArtifact>) -> Option<&'a str> {
        Some(data.value())
    }

    fn locate(
        range: Range<usize>,
        data: &MetadataData,
        _artifact: Option<&NoArtifact>,
    ) -> Option<MetadataLocation> {
        Some(MetadataLocation::new(data.key.clone()).with_span(range))
    }
}
