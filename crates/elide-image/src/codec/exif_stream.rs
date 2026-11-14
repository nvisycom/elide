//! [`ExifStream`]: an image's surfaced EXIF fields, the metadata part beside
//! the pixel stream.
//!
//! Streams each privacy-relevant EXIF field as a [`Metadata`] chunk labelled
//! with what it is, applies each redaction to the field's value (the whole
//! value, or the part a span narrows to), and writes the edited fields to the
//! image's shared buffer, a field left blank removed. The document then
//! encodes the buffer once, carrying both the pixel and the metadata edits.

use elide_codec::metadata::MetadataFields;
use elide_codec::{FlaggedItem, FormatId, Stream};
use elide_core::Result;
use elide_core::modality::metadata::{Metadata, MetadataData, MetadataLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;

use super::image_state::ImageState;

/// The EXIF metadata part: yields one chunk per surfaced field and edits the
/// fields in place on the shared [`ImageState`].
pub(super) struct ExifStream {
    state: ImageState,
    format_id: FormatId,
    /// Every surfaced field, read once at decode and kept in document order.
    fields: MetadataFields,
}

impl ExifStream {
    /// The EXIF stream over `state`, reading its fields once.
    pub(super) fn new(state: ImageState, format_id: FormatId) -> Result<Self> {
        let fields = MetadataFields::new(state.metadata_fields()?);
        Ok(Self {
            state,
            format_id,
            fields,
        })
    }
}

impl Stream<Metadata> for ExifStream {
    fn format(&self) -> FormatId {
        self.format_id
    }

    fn chunks(&self) -> Result<Vec<Chunk<Metadata>>> {
        Ok(self.fields.chunks())
    }

    fn flagged(&self) -> Result<Vec<FlaggedItem<Metadata>>> {
        Ok(self.fields.flagged())
    }
}

#[async_trait::async_trait]
impl DataReader<Metadata> for ExifStream {
    async fn read_at(&self, location: &MetadataLocation) -> Result<MetadataData> {
        self.fields.read_at(location)
    }
}

#[async_trait::async_trait]
impl DataWriter<Metadata> for ExifStream {
    async fn write_at(&mut self, redactions: Redactions<Metadata>) -> Result<()> {
        let edits = self.fields.redact(redactions)?;
        let edits: Vec<(&str, &str)> = edits
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        self.state.edit_metadata(&edits)
    }
}
