//! [`ImageState`]: the decoded image shared between the pixel stream and the
//! EXIF stream.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use elide_core::Result;
use elide_core::modality::metadata::MetadataData;
use elide_core::redaction::Redactions;

use crate::ImageBuffer;
use crate::modality::{Image, ImageData, ImageLocation};

/// The decoded image, shared between the pixel stream and the EXIF stream, which
/// both edit the one buffer: a redaction on either is in the image the
/// native document encodes from the pixel stream. `Clone` shares the one buffer (an
/// `Arc` bump); the lock is held only inside these methods.
#[derive(Clone)]
pub(super) struct ImageState(Arc<Mutex<ImageBuffer>>);

impl ImageState {
    /// Wrap a decoded image buffer.
    pub(super) fn new(buffer: ImageBuffer) -> Self {
        Self(Arc::new(Mutex::new(buffer)))
    }

    /// The location of the whole image.
    pub(super) fn frame(&self) -> ImageLocation {
        self.0.lock().unwrap().frame()
    }

    /// Encode the current pixels to bytes with the source's metadata.
    pub(super) fn encode(&self) -> Result<Bytes> {
        self.0.lock().unwrap().encode()
    }

    /// Each surfaced EXIF field the image carries.
    pub(super) fn metadata_fields(&self) -> Result<Vec<MetadataData>> {
        self.0.lock().unwrap().metadata_fields()
    }

    /// Write each EXIF field's new value in place, a blank value removing the
    /// field.
    pub(super) fn edit_metadata(&self, fields: &[(&str, &str)]) -> Result<()> {
        self.0.lock().unwrap().edit_metadata(fields)
    }

    /// The whole image as an [`ImageData`]; see [`ImageBuffer::data`].
    pub(super) fn data(&self) -> Result<ImageData> {
        self.0.lock().unwrap().data()
    }

    /// The region `location` addresses; see [`ImageBuffer::read_at`].
    pub(super) fn read_at(&self, location: &ImageLocation) -> Result<ImageData> {
        self.0.lock().unwrap().read_at(location)
    }

    /// Redact every region in `redactions`, in place; see
    /// [`ImageBuffer::redact_all`].
    pub(super) fn redact(&self, redactions: Redactions<Image>) -> Result<()> {
        self.0.lock().unwrap().redact_all(redactions).map(drop)
    }
}
