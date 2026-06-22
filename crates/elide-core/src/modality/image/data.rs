//! [`ImageData`]: the encoded-image payload for the [`Image`] modality.
//!
//! [`Image`]: super::Image

use bytes::Bytes;
use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::ModalityData;
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
pub struct ImageData {
    /// Encoded image bytes. Skipped by serde: the bytes are the raw payload,
    /// not metadata, and a serialized report (entities, provenance) has no
    /// need to carry megabytes of image data.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub bytes: Bytes,
    /// Pixel dimensions of the encoded image.
    pub dimensions: Dimensions,
    /// Original filename, when known.
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

    /// Lowercased extension derived from [`filename`],
    /// or `"png"` when no filename is set or it has no extension.
    ///
    /// [`filename`]: Self::filename
    pub fn extension(&self) -> &str {
        self.filename
            .as_deref()
            .and_then(|name| name.rsplit_once('.'))
            .map(|(_, ext)| ext)
            .unwrap_or("png")
    }
}

impl ModalityData for ImageData {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_falls_back_to_png() {
        let d = ImageData::new(Bytes::new(), Dimensions::new(10, 10));
        assert_eq!(d.extension(), "png");
        let named = d.with_filename("scan.JPEG");
        assert_eq!(named.extension(), "JPEG");
    }
}
