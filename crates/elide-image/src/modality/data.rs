//! [`ImageData`]: the encoded-image payload for the [`Image`] modality.
//!
//! [`Image`]: super::Image

use std::path::Path;

use bytes::Bytes;
use elide_core::modality::ModalityData;
use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::ImageFormat;
use crate::primitive::Dimensions;

/// Per-call payload a recognizer inspects for the [`Image`] modality.
///
/// Carries the encoded bytes plus the pixel [`Dimensions`], which a
/// recognizer that emits unit-square boxes needs to scale them into pixel
/// coordinates. An optional filename aids diagnostics and encoding
/// inference.
///
/// [`Image`]: super::Image
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ImageData {
    /// Encoded image bytes. Skipped by serde: the bytes are the raw payload,
    /// not metadata, and a serialized report (entities, provenance) has no
    /// need to carry megabytes of image data.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub bytes: Bytes,
    /// Pixel dimensions of the encoded image.
    pub dimensions: Dimensions,
    /// Original filename, when known.
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub filename: Option<HipStr<'static>>,
}

impl ImageData {
    /// Wrap encoded bytes and their pixel dimensions; filename unset.
    pub fn new(bytes: impl Into<Bytes>, dimensions: Dimensions) -> Self {
        Self {
            bytes: bytes.into(),
            dimensions,
            filename: None,
        }
    }

    /// Attach an original filename.
    #[must_use]
    pub fn with_filename(mut self, filename: impl Into<HipStr<'static>>) -> Self {
        self.filename = Some(filename.into());
        self
    }

    /// The image format the [`filename`] extension names, or `None` when there is
    /// no filename, it has no extension, or the extension is not a supported
    /// format.
    ///
    /// A filename-based *hint* only: the authoritative format is what
    /// [`ImageBuffer::open`](crate::ImageBuffer::open) detects from the bytes. A
    /// caller that needs a definite format decodes the bytes rather than trusting
    /// the name.
    ///
    /// [`filename`]: Self::filename
    #[must_use]
    pub fn format(&self) -> Option<ImageFormat> {
        let extension = self
            .filename
            .as_deref()
            .and_then(|name| Path::new(name).extension())
            .and_then(|e| e.to_str())?;
        ImageFormat::from_extension(extension)
    }
}

impl ModalityData for ImageData {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_is_none_without_a_known_extension() {
        // No filename: no format hint (do not guess a default).
        let d = ImageData::new(Bytes::new(), Dimensions::new(10, 10));
        assert_eq!(d.format(), None);
        // An unknown extension is never a supported format.
        assert_eq!(d.with_filename("scan.bmp").format(), None);
    }

    #[cfg(feature = "jpeg")]
    #[test]
    fn a_known_extension_maps_to_the_typed_format() {
        // Case-insensitive; requires the format's feature to be enabled.
        let d = ImageData::new(Bytes::new(), Dimensions::new(10, 10)).with_filename("scan.JPEG");
        assert_eq!(d.format(), Some(ImageFormat::Jpeg));
    }
}
