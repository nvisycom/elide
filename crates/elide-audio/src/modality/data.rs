//! [`AudioData`]: the encoded-audio payload for the [`Audio`] modality.
//!
//! [`Audio`]: super::Audio

use std::path::Path;

use bytes::Bytes;
use elide_core::modality::ModalityData;
use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::AudioFormat;

/// Per-call payload a recognizer inspects for the [`Audio`] modality.
///
/// Carries the encoded audio bytes; an optional filename aids diagnostics
/// and encoding inference (the container format a decoder should expect).
/// The recognizable text, a timestamped transcript, is *not* held here;
/// a speech-to-text [`Enricher`] stamps it onto the call's
/// [`artifact`], keeping
/// `AudioData` the codec's payload alone.
///
/// [`Audio`]: super::Audio
/// [`Enricher`]: elide_core::enrichment::Enricher
/// [`artifact`]: elide_core::recognition::RecognizerContext::artifact
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AudioData {
    /// Encoded audio bytes. Skipped by serde: the bytes are the raw payload,
    /// not metadata, and a serialized report has no need to carry the audio
    /// stream.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub bytes: Bytes,
    /// Original filename, when known.
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub filename: Option<HipStr<'static>>,
}

impl AudioData {
    /// Wrap encoded audio bytes; filename unset.
    pub fn new(bytes: impl Into<Bytes>) -> Self {
        Self {
            bytes: bytes.into(),
            filename: None,
        }
    }

    /// Attach an original filename.
    #[must_use]
    pub fn with_filename(mut self, filename: impl Into<HipStr<'static>>) -> Self {
        self.filename = Some(filename.into());
        self
    }

    /// The audio format the [`filename`] extension names, or `None` when there is
    /// no filename, it has no extension, or the extension is not a format this
    /// build supports.
    ///
    /// A filename-based *hint* only: the authoritative format is what
    /// [`AudioBuffer::open`](crate::AudioBuffer::open) detects from the bytes. A
    /// caller that needs a definite format decodes the bytes rather than trusting
    /// the name.
    ///
    /// [`filename`]: Self::filename
    #[must_use]
    pub fn format(&self) -> Option<AudioFormat> {
        let extension = self
            .filename
            .as_deref()
            .and_then(|name| Path::new(name).extension())
            .and_then(|e| e.to_str())?;
        AudioFormat::from_extension(extension)
    }
}

impl ModalityData for AudioData {}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "mp3")]
    #[test]
    fn format_is_none_without_a_known_extension() {
        // No filename: no format hint (do not guess a default).
        let d = AudioData::new(Bytes::new());
        assert_eq!(d.format(), None);
        // An unknown extension is not a supported format.
        assert_eq!(d.clone().with_filename("call.aac").format(), None);
        // A known extension maps to the typed format, case-insensitively.
        assert_eq!(d.with_filename("call.MP3").format(), Some(AudioFormat::Mp3));
    }
}
