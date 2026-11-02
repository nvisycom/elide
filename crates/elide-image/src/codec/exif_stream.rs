//! [`ExifStream`]: an image's surfaced EXIF fields, the metadata part beside
//! the pixel stream.
//!
//! Streams each privacy-relevant EXIF field as a [`Metadata`] chunk labelled
//! with what it is, and applies each field's redaction to the image's shared
//! buffer: a removal clears the field, an overwrite writes its new text. The
//! recombiner then encodes the buffer once, carrying both the pixel and the
//! metadata edits.

use elide_codec::content::ContentData;
use elide_codec::{FormatId, Stream};
use elide_core::Result;
use elide_core::modality::metadata::{
    Metadata, MetadataData, MetadataLocation, MetadataReplacement,
};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;

use super::image_state::ImageState;

/// The EXIF metadata part: yields one chunk per surfaced field and edits the
/// fields in place on the shared [`ImageState`].
pub(super) struct ExifStream {
    state: ImageState,
    format_id: FormatId,
    /// Every surfaced field, read once at decode and kept in document order:
    /// the chunks a caller sees and the set [`read_at`](DataReader::read_at)
    /// looks one up in without re-parsing.
    fields: Vec<MetadataData>,
}

impl ExifStream {
    /// The EXIF stream over `state`, reading its fields once.
    pub(super) fn new(state: ImageState, format_id: FormatId) -> Result<Self> {
        let fields = state.metadata_fields()?;
        Ok(Self {
            state,
            format_id,
            fields,
        })
    }
}

impl Stream<Metadata> for ExifStream {
    fn format(&self) -> FormatId {
        self.format_id.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        Ok(ContentData::new(self.state.source_bytes()))
    }

    fn chunks(&self) -> Result<Vec<Chunk<Metadata>>> {
        Ok(self
            .fields
            .iter()
            .map(|field| Chunk::new(MetadataLocation::new(field.key.clone()), field.clone()))
            .collect())
    }
}

#[async_trait::async_trait]
impl DataReader<Metadata> for ExifStream {
    async fn read_at(&self, location: &MetadataLocation) -> Result<Option<MetadataData>> {
        let key = location.key.as_str();
        Ok(self.fields.iter().find(|field| field.key == key).cloned())
    }
}

#[async_trait::async_trait]
impl DataWriter<Metadata> for ExifStream {
    async fn write_at(&mut self, redactions: Redactions<Metadata>) -> Result<()> {
        let mut removed = Vec::new();
        let mut replaced = Vec::new();
        for (location, replacement) in redactions.into_iter() {
            match replacement {
                MetadataReplacement::Replace(value) => replaced.push((location.key, value)),
                // Removal, and any treatment added later, clears the field:
                // nothing leaves it intact.
                _ => removed.push(location.key),
            }
        }
        let removed: Vec<&str> = removed.iter().map(|key| key.as_str()).collect();
        let replaced: Vec<(&str, &str)> = replaced
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        self.state.edit_metadata(&removed, &replaced)
    }
}
