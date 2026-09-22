//! [`SttRequest`]: one per-call speech-to-text request handed to an
//! [`SttBackend`].
//!
//! [`SttBackend`]: super::SttBackend

use elide_core::primitive::LanguageTag;
use uuid::Uuid;

/// One per-call STT request handed to an [`SttBackend`].
///
/// Bundles the audio bytes with advisory hints (filename, language,
/// correlation id). Borrowed (`SttRequest<'a>`) so call sites that already
/// own the underlying values hand them through without cloning.
///
/// [`SttBackend`]: super::SttBackend
#[derive(Debug, Clone)]
pub struct SttRequest<'a> {
    /// Raw audio bytes (WAV, MP3, FLAC, …). The backend honours whatever
    /// container and codec it accepts; returned segment timings refer back
    /// into this clip.
    pub audio: &'a [u8],
    /// Original filename, when known. Some backends use the extension for
    /// MIME-type detection on multipart uploads.
    pub filename: Option<&'a str>,
    /// Caller-asserted language. Backends that support per-call language
    /// hinting use this to pick a model variant; others ignore it.
    pub language: Option<&'a LanguageTag>,
    /// Per-call correlation id propagated to remote backends for tracing.
    pub correlation_id: Option<Uuid>,
}

impl<'a> SttRequest<'a> {
    /// A request over `audio` with no advisory hints set.
    pub fn new(audio: &'a [u8]) -> Self {
        Self {
            audio,
            filename: None,
            language: None,
            correlation_id: None,
        }
    }

    /// Builder-style setter for the original filename.
    #[must_use]
    pub fn with_filename(mut self, filename: &'a str) -> Self {
        self.filename = Some(filename);
        self
    }

    /// Builder-style setter for the language hint.
    #[must_use]
    pub fn with_language(mut self, language: &'a LanguageTag) -> Self {
        self.language = Some(language);
        self
    }

    /// Builder-style setter for the correlation id.
    #[must_use]
    pub fn with_correlation_id(mut self, correlation_id: Uuid) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }
}
