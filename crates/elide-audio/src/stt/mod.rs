//! Speech-to-text: transcribe an audio clip and enrich the call with a
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
//! recognizer can read it. The no-op `MockBackend` is behind `test-utils`.

mod enricher;
#[cfg(any(test, feature = "test-utils"))]
mod mock;
mod request;
mod response;

use elide_core::Result;
use elide_core::entity::audit::ModelEvent;

pub use self::enricher::{SttEnricher, SttEnricherBuilder};
#[cfg(any(test, feature = "test-utils"))]
#[cfg_attr(docsrs, doc(cfg(feature = "test-utils")))]
pub use self::mock::MockBackend;
pub use self::request::SttRequest;
pub use self::response::SttResponse;

/// Per-call speech-to-text backend.
///
/// Implemented by everything that turns `(audio, language?)` into
/// transcribed segments, hosted provider clients (OpenAI Whisper,
/// Deepgram, AssemblyAI), local model wrappers, and the in-process no-op
/// test stub. One trait covers every flavour: providers that emit a single
/// full-clip segment, providers that emit diarized multi-speaker segments,
/// and providers that emit word-level timings.
///
/// Confidence values **must** be normalised to `0.0..=1.0` before being
/// placed on a segment or word. Backends whose upstream API uses a
/// different scale convert before returning.
///
/// Object-safe: extractors hold `Arc<dyn SttBackend>` and dispatch per
/// call.
#[async_trait::async_trait]
pub trait SttBackend: Send + Sync + 'static {
    /// Backend identity (model / service name + provenance detail).
    ///
    /// Identifies the actual model the backend wraps (e.g. `"noop-stt"`),
    /// stamped into the provenance of every entity detected over the
    /// transcript so the audit records which STT pass produced it.
    fn provenance(&self) -> ModelEvent;

    /// Transcribe `request` into ordered segments.
    ///
    /// # Errors
    ///
    /// Returns the underlying transport / parse / inference error.
    async fn transcribe(&self, request: SttRequest<'_>) -> Result<SttResponse>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_returns_empty() {
        let backend = MockBackend::new();
        let audio = vec![0u8; 8];
        let response = backend.transcribe(SttRequest::new(&audio)).await.unwrap();
        assert!(response.segments.is_empty());
    }
}
