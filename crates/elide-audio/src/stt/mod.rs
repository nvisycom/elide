//! Speech-to-text: transcribe an audio clip.
//!
//! Enriches the call with a
//! [`Transcription`](crate::modality::Transcription).
//!
//! The [`SttBackend`] trait covers every STT engine, hosted APIs that emit a
//! single full-clip segment (OpenAI Whisper), hosted APIs that emit diarized
//! multi-speaker segments (Deepgram, AssemblyAI), local/self-hosted inference
//! services, and the in-process test stub. Each backend turns a request (audio
//! bytes + optional hints) into a response of ordered
//! [`TranscriptSegment`](crate::modality::TranscriptSegment)s, so its output
//! drops straight onto the call's artifacts with no remapping. The
//! [`SttEnricher`] drives a backend per call and stamps the recognized
//! [`Transcription`](crate::modality::Transcription) onto the audio so a
//! recognizer can read it. The no-op `MockBackend` is behind `mocks`.

mod enricher;
#[cfg(any(test, feature = "mocks"))]
mod mock;
mod request;
mod response;

use elide_core::backend::Backend;

pub use self::enricher::{SttEnricher, SttEnricherBuilder};
#[cfg(any(test, feature = "mocks"))]
#[cfg_attr(docsrs, doc(cfg(feature = "mocks")))]
pub use self::mock::MockBackend;
pub use self::request::SttRequest;
pub use self::response::SttResponse;

/// A speech-to-text [`Backend`](elide_core::backend::Backend): `Backend<Request<'a>
/// = SttRequest<'a>, Response = SttResponse>`.
///
/// A shorthand bound so the enricher holds any STT backend generically.
/// Everything that turns `(audio, language?)` into transcribed segments
/// implements it by implementing `Backend` with these associated types — hosted
/// provider clients (Whisper, Deepgram, AssemblyAI), local model wrappers, and
/// the in-process no-op test stub.
///
/// Confidence values **must** be normalised to `0.0..=1.0` before being placed
/// on a segment or word; backends whose upstream API uses a different scale
/// convert before returning.
pub trait SttBackend:
    for<'a> Backend<Request<'a> = SttRequest<'a>, Response = SttResponse>
{
}

impl<B> SttBackend for B where
    B: for<'a> Backend<Request<'a> = SttRequest<'a>, Response = SttResponse>
{
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modality::AudioFormat;

    #[tokio::test]
    async fn mock_returns_empty() {
        let backend = MockBackend::new();
        let audio = vec![0u8; 8];
        let request = SttRequest {
            audio: &audio,
            format: AudioFormat::Wav,
            language: None,
            correlation_id: None,
        };
        let response = Backend::call(&backend, request).await.unwrap();
        assert!(response.segments.is_empty());
    }
}
