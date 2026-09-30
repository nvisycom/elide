//! [`SttRequest`]: one per-call speech-to-text request handed to an
//! [`SttBackend`].
//!
//! [`SttBackend`]: super::SttBackend

use elide_core::backend::BackendRequest;
use elide_core::primitive::LanguageTag;
use uuid::Uuid;

use crate::modality::AudioFormat;

/// One per-call STT request handed to an [`SttBackend`].
///
/// Bundles the audio bytes with the format the caller knows from ingestion,
/// plus optional language and correlation hints. Borrowed (`SttRequest<'a>`) so
/// call sites that already own the underlying values hand them through without
/// cloning. Constructed as a struct literal — the format is always known by the
/// time STT runs.
///
/// [`SttBackend`]: super::SttBackend
#[derive(Debug, Clone)]
pub struct SttRequest<'a> {
    /// Raw audio bytes (WAV, MP3, FLAC, …). The backend honours whatever
    /// container and codec it accepts; returned segment timings refer back
    /// into this clip.
    pub audio: &'a [u8],
    /// The container format. Known losslessly from ingestion (a self-describing
    /// container had to be identified to read it), so a remote backend that
    /// never decodes the bytes takes it from here rather than sniffing.
    pub format: AudioFormat,
    /// Caller-asserted language. Backends that support per-call language
    /// hinting use this to pick a model variant; others ignore it.
    pub language: Option<&'a LanguageTag>,
    /// Per-call correlation id propagated to remote backends for tracing.
    pub correlation_id: Option<Uuid>,
}

impl BackendRequest for SttRequest<'_> {}
