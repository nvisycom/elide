//! [`ImageData`]: the decoded-image payload for the [`Image`] modality.
//!
//! [`Image`]: super::Image

use std::sync::Arc;

use bytes::Bytes;
use elide_core::modality::ModalityData;

use super::ImageFormat;
use crate::RasterImage;
use crate::primitive::Dimensions;

/// Per-call payload a recognizer inspects for the [`Image`] modality: the image
/// decoded once, plus its original container bytes.
///
/// The image is decoded at ingestion and shared here, so [`format`] and
/// [`dimensions`] are intrinsic facts read for free — no consumer re-decodes to
/// recover them, and none can drift, because the decode is immutable and shared
/// (`Arc<RasterImage>`, so a clone is an `Arc` bump, not a pixel copy). The
/// [`source`] bytes are the original container, retained so redaction stays
/// byte-faithful (an unchanged image emits its own bytes) and metadata (EXIF)
/// reads the real container.
///
/// This is the immutable read view a recognizer sees; the mutable, in-place
/// redaction target is [`ImageBuffer`], which the codec owns.
///
/// [`Image`]: super::Image
/// [`format`]: ImageData::format
/// [`dimensions`]: ImageData::dimensions
/// [`source`]: ImageData::source
/// [`ImageBuffer`]: crate::ImageBuffer
#[derive(Debug, Clone)]
pub struct ImageData {
    /// The decoded image, shared so a clone is an `Arc` bump.
    image: Arc<RasterImage>,
    /// The original container bytes, for byte-faithful passthrough and metadata.
    source: Bytes,
}

impl ImageData {
    /// Wrap a decoded image and its original container bytes.
    #[must_use]
    pub fn new(image: RasterImage, source: Bytes) -> Self {
        Self {
            image: Arc::new(image),
            source,
        }
    }

    /// The decoded image.
    #[must_use]
    pub fn image(&self) -> &RasterImage {
        &self.image
    }

    /// The original container bytes.
    #[must_use]
    pub fn source(&self) -> &Bytes {
        &self.source
    }

    /// The image's format.
    #[must_use]
    pub fn format(&self) -> ImageFormat {
        self.image.format()
    }

    /// The image's pixel dimensions.
    #[must_use]
    pub fn dimensions(&self) -> Dimensions<u32> {
        self.image.dimensions()
    }
}

impl ModalityData for ImageData {}
