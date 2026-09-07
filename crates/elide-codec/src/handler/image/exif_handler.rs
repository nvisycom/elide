//! Handler and loader for an image's EXIF metadata, the `#exif` sub-part.
//!
//! An image's pixel handler is a [`Container`](crate::Container) that exposes
//! its EXIF as a nested `#exif` sub-part whose bytes are the whole image, the
//! same way a DOCX exposes its embedded images. This handler decodes that part
//! as the [`Metadata`] modality: it streams each privacy-relevant EXIF field as
//! a chunk for the recognizer, records the fields picked for removal via
//! [`write_at`], and re-encodes the image with exactly those fields stripped.
//! The parent image handler then adopts those stripped bytes and folds its own
//! pixel redactions on top, so the image is written once.
//!
//! [`Metadata`]: elide_core::modality::metadata::Metadata
//! [`write_at`]: elide_core::modality::DataWriter::write_at

use elide_core::Result;
use elide_core::modality::metadata::{Metadata, MetadataData, MetadataLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;
use elide_image::ImageBuffer;

use crate::content::ContentData;
use crate::{FormatId, Handler, Loader};

/// Stable [`FormatId`](crate::FormatId) for the image-metadata sub-part.
pub const FORMAT_ID: crate::FormatId = crate::FormatId::new("elide.image.exif");

/// Handler for an image's EXIF metadata: streams fields, removes chosen ones,
/// encodes the metadata-stripped image.
#[derive(Debug)]
pub(crate) struct ExifHandler {
    /// The decoded image, holding the source container the EXIF lives in.
    buffer: ImageBuffer,
    /// The fields yet to stream, drained by `read_next` (reversed for pop).
    pending: Vec<MetadataData>,
    /// Keys picked for removal, applied on `encode`.
    removed: Vec<String>,
}

impl ExifHandler {
    /// Wrap a decoded image, priming its fields for streaming.
    pub(crate) fn new(buffer: ImageBuffer) -> Result<Self> {
        let mut pending = buffer.metadata_fields()?;
        pending.reverse(); // popped, so reverse for first-field-first order
        Ok(Self {
            buffer,
            pending,
            removed: Vec::new(),
        })
    }
}

#[::async_trait::async_trait]
impl Handler<Metadata> for ExifHandler {
    fn format(&self) -> FormatId {
        FORMAT_ID.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        let keys: Vec<&str> = self.removed.iter().map(String::as_str).collect();
        Ok(ContentData::new(self.buffer.strip_metadata_keys(&keys)?))
    }

    async fn read_next(&mut self) -> Result<Option<Chunk<Metadata>>> {
        let Some(field) = self.pending.pop() else {
            return Ok(None);
        };
        let location = MetadataLocation::new(field.key.clone());
        Ok(Some(Chunk::new(location, field)))
    }
}

#[::async_trait::async_trait]
impl DataReader<Metadata> for ExifHandler {
    async fn read_at(&self, location: &MetadataLocation) -> Result<Option<MetadataData>> {
        let key = location.key.as_str();
        Ok(self
            .buffer
            .metadata_fields()?
            .into_iter()
            .find(|field| field.key == key))
    }
}

#[::async_trait::async_trait]
impl DataWriter<Metadata> for ExifHandler {
    async fn write_at(&mut self, redactions: Redactions<Metadata>) -> Result<()> {
        for (location, _replacement) in redactions.into_iter() {
            // Every treatment clears the field: this container can only drop an
            // EXIF tag, not rewrite it, so a `Replace` degrades to a clear rather
            // than shipping the original value. Clearing unconditionally is
            // fail-closed against any future `MetadataReplacement` variant, no
            // treatment leaves the field intact.
            self.removed.push(location.key.as_str().to_owned());
        }
        Ok(())
    }
}

/// Loader that decodes image bytes into a [`ExifHandler`].
#[derive(Debug)]
pub(crate) struct ExifLoader;

#[::async_trait::async_trait]
impl Loader<Metadata> for ExifLoader {
    type Handler = ExifHandler;

    async fn decode(&self, content: ContentData) -> Result<ExifHandler> {
        ExifHandler::new(ImageBuffer::open(content.as_bytes())?)
    }
}

/// The pseudo-extension the image handler's `#exif` sub-part is decoded with;
/// it resolves this format in the registry (the fold looks a part up by its
/// hint as an extension).
pub const EXIF_HINT: &str = "x-elide-exif";

/// [`Format`](crate::Format) descriptor for the image-metadata sub-part.
pub fn format() -> crate::Format {
    crate::Format::new::<Metadata, _>(FORMAT_ID.clone(), ExifLoader)
        .with_extensions([EXIF_HINT])
        .with_content_types(["application/x-elide-image-metadata"])
}
