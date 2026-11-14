//! [`ImageState`]: the decoded image shared between the pixel stream and the
//! EXIF stream.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use elide_core::modality::metadata::MetadataData;
use elide_core::redaction::Redactions;
use elide_core::{Error, Result};

use crate::ImageBuffer;
use crate::modality::{Image, ImageData, ImageLocation};
use crate::primitive::Dimensions;

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

    /// The image's pixel dimensions.
    pub(super) fn dimensions(&self) -> Dimensions<u32> {
        self.0.lock().unwrap().dimensions()
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

    /// The whole decoded frame as an [`ImageData`]: the decoded pixels plus the
    /// re-encoded bytes as its source.
    pub(super) fn image_data(&self) -> Result<ImageData> {
        let buffer = self.0.lock().unwrap();
        let source = buffer.encode()?;
        Ok(ImageData::new(buffer.raster().clone(), source))
    }

    /// Crop the region `location` addresses into an [`ImageData`].
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] when the region falls outside the image.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub(super) fn crop_encode(&self, location: &ImageLocation) -> Result<ImageData> {
        let buffer = self.0.lock().unwrap();
        let cropped = location
            .bounding_box
            .to_pixels(buffer.dimensions())
            .and_then(|region| buffer.crop(region))
            .ok_or_else(outside_the_image)?;
        let source = cropped.encode()?;
        Ok(ImageData::new(cropped, source))
    }

    /// Redact every region in `redactions`, in place.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] when a region falls outside the image; nothing is
    /// painted then.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub(super) fn redact(&self, redactions: Redactions<Image>) -> Result<()> {
        let mut buffer = self.0.lock().unwrap();
        let dims = buffer.dimensions();
        let regions = redactions
            .into_iter()
            .map(|(location, replacement)| {
                let region = location
                    .bounding_box
                    .to_pixels(dims)
                    .ok_or_else(outside_the_image)?;
                Ok((region, replacement))
            })
            .collect::<Result<Vec<_>>>()?;
        for (region, replacement) in regions {
            buffer.redact(region, &replacement);
        }
        Ok(())
    }
}

/// The error for a region no pixel of the image falls in.
fn outside_the_image() -> Error {
    Error::malformed("the region falls outside the image")
}
