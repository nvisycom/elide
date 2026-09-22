//! [`AudioFormat`]: the container formats this crate supports.

/// The container format an [`AudioBuffer`](crate::AudioBuffer) was opened as,
/// and re-encodes to.
///
/// A closed set: each variant is present only when its format feature is
/// enabled, so a build cannot name a format it cannot decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum AudioFormat {
    /// WAV (RIFF), decoded and re-written sample-for-sample by `hound`.
    #[cfg(feature = "wav")]
    Wav,
    /// MP3, decoded to PCM by `symphonia` and re-encoded by LAME.
    #[cfg(feature = "mp3")]
    Mp3,
}

impl AudioFormat {
    /// The format a filename `extension` names (case-insensitive), or `None` when
    /// it is not a format this build supports.
    ///
    /// A filename-based *hint* only: the authoritative format is what
    /// [`AudioBuffer::open`](crate::AudioBuffer::open) detects from the bytes. An
    /// extension whose format feature is not enabled maps to `None`.
    #[must_use]
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            #[cfg(feature = "wav")]
            "wav" => Some(AudioFormat::Wav),
            #[cfg(feature = "mp3")]
            "mp3" => Some(AudioFormat::Mp3),
            _ => None,
        }
    }
}
