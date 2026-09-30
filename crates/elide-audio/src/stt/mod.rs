//! Speech-to-text: transcribe an audio clip.
//!
//! Enriches the call with a
//! [`Transcription`](crate::modality::Transcription).
//!
//! An STT backend is a [`Backend`](elide_core::backend::Backend) whose request is
//! [`SttRequest`] and whose response is [`SttResponse`]. It covers every STT
//! engine: hosted APIs that emit a single full-clip segment (OpenAI Whisper),
//! hosted APIs that emit diarized multi-speaker segments (Deepgram, AssemblyAI),
//! local/self-hosted inference services, and the in-process test stub. Each turns
//! a request (audio bytes + optional hints) into a response of ordered
//! [`TranscriptSegment`](crate::modality::TranscriptSegment)s, so its output
//! drops straight onto the call's artifacts with no remapping. The
//! [`SttEnricher`] drives a backend per call and stamps the recognized
//! [`Transcription`](crate::modality::Transcription) onto the audio so a
//! recognizer can read it. The no-op `MockBackend` is behind `mocks`.
//!
//! Confidence values **must** be normalised to `0.0..=1.0` before being placed on
//! a segment or word; a backend whose upstream API uses a different scale converts
//! before returning.

mod enricher;
#[cfg(any(test, feature = "mocks"))]
mod mock;
mod request;
mod response;

pub use self::enricher::SttEnricher;
#[cfg(any(test, feature = "mocks"))]
#[cfg_attr(docsrs, doc(cfg(feature = "mocks")))]
pub use self::mock::MockBackend;
pub use self::request::SttRequest;
pub use self::response::SttResponse;

#[cfg(test)]
mod tests {
    use elide_core::backend::Backend;

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
