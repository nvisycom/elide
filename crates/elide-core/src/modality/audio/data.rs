//! [`AudioData`]: the encoded-audio payload for the [`Audio`] modality.
//!
//! [`Audio`]: super::Audio

use bytes::Bytes;
use hipstr::HipStr;
#[cfg(feature = "schema")]
use schemars::JsonSchema;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::{ModalityData, extension_or};

/// Per-call payload a recognizer inspects for the [`Audio`] modality.
///
/// Carries the encoded audio bytes; an optional filename aids diagnostics
/// and encoding inference (the container format a decoder should expect).
/// The recognizable text — a timestamped transcript — is *not* held here;
/// a speech-to-text [`Enricher`] stamps it onto the call's
/// [`artifacts`], keeping
/// `AudioData` the codec's payload alone.
///
/// [`Audio`]: super::Audio
/// [`Enricher`]: crate::recognition::Enricher
/// [`artifacts`]: crate::recognition::RecognizerContext::artifacts
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
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

    /// Extension derived from [`filename`], or `"wav"` when no filename is
    /// set or it has no extension.
    ///
    /// [`filename`]: Self::filename
    pub fn extension(&self) -> &str {
        extension_or(self.filename.as_deref(), "wav")
    }
}

impl ModalityData for AudioData {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_falls_back_to_wav() {
        let d = AudioData::new(Bytes::new());
        assert_eq!(d.extension(), "wav");
        let named = d.with_filename("call.MP3");
        assert_eq!(named.extension(), "MP3");
    }
}
