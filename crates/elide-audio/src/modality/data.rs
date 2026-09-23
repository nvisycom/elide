//! [`AudioData`]: the encoded-audio payload for the [`Audio`] modality.
//!
//! [`Audio`]: super::Audio

use bytes::Bytes;
use elide_core::modality::ModalityData;

/// Per-call payload a recognizer inspects for the [`Audio`] modality: the
/// encoded audio bytes.
///
/// The bytes are a complete, self-describing container, so a consumer recovers
/// the format (by sniffing) and the samples (by decoding) from them via
/// [`AudioBuffer::open`](crate::AudioBuffer::open); nothing is cached alongside.
/// The recognizable text, a timestamped transcript, is *not* held here; a
/// speech-to-text [`Enricher`] stamps it onto the call's [`artifact`], keeping
/// `AudioData` the codec's payload alone.
///
/// [`Audio`]: super::Audio
/// [`Enricher`]: elide_core::enrichment::Enricher
/// [`artifact`]: elide_core::recognition::Subject::artifact
#[derive(Debug, Clone)]
pub struct AudioData {
    /// Encoded audio bytes.
    pub bytes: Bytes,
}

impl AudioData {
    /// Wrap encoded audio bytes.
    pub fn new(bytes: impl Into<Bytes>) -> Self {
        Self {
            bytes: bytes.into(),
        }
    }
}

impl ModalityData for AudioData {}
