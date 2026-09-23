//! [`ImageData`]: the encoded-image payload for the [`Image`] modality.
//!
//! [`Image`]: super::Image

use bytes::Bytes;
use elide_core::modality::ModalityData;

/// Per-call payload a recognizer inspects for the [`Image`] modality: the
/// encoded image bytes.
///
/// The bytes are a complete, self-describing container (PNG/JPEG/TIFF), so a
/// consumer recovers everything else from them: the format by sniffing the
/// magic bytes, the pixel dimensions and the pixels by decoding (both via
/// [`ImageBuffer::open`](crate::ImageBuffer::open)). Nothing is cached
/// alongside — a cached dimension or format could drift from the bytes, and
/// every real consumer already decodes.
///
/// [`Image`]: super::Image
#[derive(Debug, Clone)]
pub struct ImageData {
    /// Encoded image bytes.
    pub bytes: Bytes,
}

impl ImageData {
    /// Wrap encoded image bytes.
    pub fn new(bytes: impl Into<Bytes>) -> Self {
        Self {
            bytes: bytes.into(),
        }
    }
}

impl ModalityData for ImageData {}
