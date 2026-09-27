//! [`ImageState`]: the decoded image shared between the pixel stream and the
//! recombiner.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use elide_core::Result;
use elide_core::redaction::Redactions;

use crate::ImageBuffer;
use crate::exif::ExifPolicy;
use crate::modality::{Image, ImageData, ImageLocation};
use crate::primitive::Dimensions;

/// The decoded image, shared between the pixel stream and the recombiner so a
/// redaction on the stream is visible when the recombiner re-encodes. `Clone`
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

    /// Encode the current pixels to bytes, applying `policy` to the metadata.
    pub(super) fn encode(&self, policy: ExifPolicy) -> Result<Bytes> {
        self.0.lock().unwrap().encode(policy)
    }

    /// Encode the current pixels laid over the metadata-stripped `container`.
    pub(super) fn encode_over_metadata(&self, container: &[u8]) -> Result<Bytes> {
        self.0.lock().unwrap().encode_over_metadata(container)
    }

    /// The whole decoded frame as an [`ImageData`]: the decoded pixels plus the
    /// re-encoded bytes as its source, under `policy`.
    pub(super) fn image_data(&self, policy: ExifPolicy) -> Result<ImageData> {
        let buffer = self.0.lock().unwrap();
        let source = buffer.encode(policy)?;
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
