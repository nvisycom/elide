//! [`AudioData`]: the encoded-audio payload for the [`Audio`] modality.
//!
//! [`Audio`]: super::Audio

use bytes::Bytes;
use elide_core::modality::ModalityData;

use super::AudioFormat;

/// Per-call payload a recognizer inspects for the [`Audio`] modality: the
/// encoded audio bytes plus the container format they were ingested as.
///
/// The [`format`] is resolved at ingestion (a self-describing container had to
/// be identified to read it) and carried here as an intrinsic fact, so a remote
/// STT backend that never decodes the bytes takes it from here rather than
/// sniffing. The recognizable text, a timestamped transcript, is *not* held
/// here; a speech-to-text [`Enricher`] stamps it onto the call's [`artifact`],
/// keeping `AudioData` the codec's payload alone.
///
/// [`Audio`]: super::Audio
/// [`format`]: AudioData::format
/// [`Enricher`]: elide_core::enrichment::Enricher
/// [`artifact`]: elide_core::recognition::Subject::artifact
#[derive(Debug, Clone)]
pub struct AudioData {
    /// Encoded audio bytes.
    pub bytes: Bytes,
    /// The container format the bytes were ingested as.
    format: AudioFormat,
}

impl AudioData {
    /// Wrap encoded audio bytes ingested as `format`.
    pub fn new(bytes: impl Into<Bytes>, format: AudioFormat) -> Self {
        Self {
            bytes: bytes.into(),
            format,
        }
    }

    /// The container format the bytes were ingested as.
    #[must_use]
    pub fn format(&self) -> AudioFormat {
        self.format
    }
}

impl ModalityData for AudioData {}
