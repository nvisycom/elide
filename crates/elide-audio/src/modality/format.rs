//! [`AudioFormat`]: the container formats this crate names.

/// The container format an [`AudioBuffer`](crate::AudioBuffer) holds, and
/// re-encodes to.
///
/// A closed set. The variants are always present — naming a format is data,
/// independent of whether this build can decode it. Whether a codec is compiled
/// in is a separate, runtime question: [`can_decode`](Self::can_decode) answers
/// it, and [`AudioBuffer::open`](crate::AudioBuffer::open) returns a capability
/// error when asked to open a format whose feature is off. This lets a caller
/// that only needs to *name* a format (a remote STT backend that never decodes
/// locally) do so in any build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum AudioFormat {
    /// WAV (RIFF), decoded and re-written sample-for-sample by `hound`.
    Wav,
    /// MP3, decoded to PCM by `symphonia` and re-encoded by LAME.
    Mp3,
}

impl AudioFormat {
    /// The format a filename `extension` names (case-insensitive), or `None`
    /// when it is not one this crate models.
    ///
    /// A filename-based *hint* only, and independent of build features: the
    /// authoritative format is the one the caller resolved at ingestion, and
    /// whether this build can decode it is [`can_decode`](Self::can_decode).
    #[must_use]
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "wav" => Some(AudioFormat::Wav),
            "mp3" => Some(AudioFormat::Mp3),
            _ => None,
        }
    }

    /// The canonical IANA media type (MIME) for this format.
    ///
    /// WAV also appears as `audio/x-wav`; this returns the standard `audio/wav`.
    #[must_use]
    pub fn mime_type(self) -> &'static str {
        match self {
            AudioFormat::Wav => "audio/wav",
            AudioFormat::Mp3 => "audio/mpeg",
        }
    }

    /// Whether this build has the codec to decode and re-encode this format.
    ///
    /// Naming a format always works; decoding needs its format feature. `open`
    /// returns a capability error for a format where this is `false`.
    #[must_use]
    pub fn can_decode(self) -> bool {
        match self {
            AudioFormat::Wav => cfg!(feature = "wav"),
            AudioFormat::Mp3 => cfg!(feature = "mp3"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_type_maps_each_format() {
        assert_eq!(AudioFormat::Wav.mime_type(), "audio/wav");
        assert_eq!(AudioFormat::Mp3.mime_type(), "audio/mpeg");
    }

    #[test]
    fn from_extension_round_trips_a_canonical_extension() {
        assert_eq!(AudioFormat::from_extension("WAV"), Some(AudioFormat::Wav));
        assert_eq!(AudioFormat::from_extension("mp3"), Some(AudioFormat::Mp3));
        assert_eq!(AudioFormat::from_extension("flac"), None);
    }
}
