//! [`ImageState`]: the decoded image shared between the pixel stream, the EXIF
//! stream, and the recombiner.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use elide_core::Result;
use elide_core::modality::metadata::MetadataData;
use elide_core::redaction::Redactions;

use crate::ImageBuffer;
use crate::modality::{Image, ImageData, ImageLocation};
use crate::primitive::Dimensions;

/// The decoded image, shared between the pixel stream, the EXIF stream, and the
/// recombiner so a redaction on either stream is visible when the recombiner
/// re-encodes. `Clone`
/// shares the one buffer (an `Arc` bump); the lock is held only inside these
/// methods.
#[derive(Clone)]
pub(super) struct ImageState(Arc<Mutex<ImageBuffer>>);

impl ImageState {
    /// Wrap a decoded image buffer.
    pub(super) fn new(buffer: ImageBuffer) -> Self {
        Self(Arc::new(Mutex::new(buffer)))
    }

    /// The image's pixel dimensions.
    pub(super) fn dimensions(&self) -> Dimensions<u32> {
        self.0.lock().unwrap().dimensions()
    }

    /// Encode the current pixels to bytes with the source's metadata.
    pub(super) fn encode(&self) -> Result<Bytes> {
        self.0.lock().unwrap().encode()
    }

    /// The source container as the metadata edits so far left it.
    pub(super) fn source_bytes(&self) -> Bytes {
        self.0.lock().unwrap().source_bytes()
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

    /// The whole decoded frame as an [`ImageData`]: the decoded pixels plus the
    /// re-encoded bytes as its source.
    pub(super) fn image_data(&self) -> Result<ImageData> {
        let buffer = self.0.lock().unwrap();
        let source = buffer.encode()?;
        Ok(ImageData::new(buffer.raster().clone(), source))
    }

    /// Crop the region `location` addresses into an [`ImageData`], or `None`
    /// when the region falls outside the image.
    pub(super) fn crop_encode(&self, location: &ImageLocation) -> Result<Option<ImageData>> {
        let buffer = self.0.lock().unwrap();
        let Some(region) = location.bounding_box.to_pixels(buffer.dimensions()) else {
            return Ok(None);
        };
        let Some(cropped) = buffer.crop(region) else {
            return Ok(None);
        };
        let source = cropped.encode()?;
        Ok(Some(ImageData::new(cropped, source)))
    }

    /// Redact every region in `redactions` that intersects the image, in place.
    pub(super) fn redact(&self, redactions: Redactions<Image>) {
        let mut buffer = self.0.lock().unwrap();
        let dims = buffer.dimensions();
        for (location, replacement) in redactions.into_iter() {
            if let Some(region) = location.bounding_box.to_pixels(dims) {
                buffer.redact(region, &replacement);
            }
        }
    }
}
