//! [`MockBackend`]: stand-in STT backend for tests, examples, and as a
//! default before a real backend is configured.

use elide_core::Result;
use elide_core::backend::Backend;
use elide_core::entity::audit::ModelEvent;

use super::{SttRequest, SttResponse};
use crate::modality::TranscriptSegment;

/// Mock STT backend: returns a fixed set of segments on every call.
///
/// Empty by default ([`new`](Self::new) / [`default`](Default::default)), the
/// no-op stub examples and offline wiring rely on, transcribing nothing. Give it
/// canned segments with [`with`](Self::with) to have every call enrich with the
/// same [`Transcription`], for tests that need a real artifact to read back.
///
/// [`Transcription`]: crate::modality::Transcription
#[derive(Debug, Default, Clone)]
pub struct MockBackend {
    segments: Vec<TranscriptSegment>,
    /// How many trailing responses `call_batch` omits; `0` honors the contract.
    dropped: usize,
}

impl MockBackend {
    /// An empty mock backend: every call transcribes nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A mock backend that returns `segments` on every call, so the enricher
    /// stamps the same [`Transcription`] onto each request.
    ///
    /// [`Transcription`]: crate::modality::Transcription
    #[must_use]
    pub fn with(segments: Vec<TranscriptSegment>) -> Self {
        Self {
            segments,
            dropped: 0,
        }
    }

    /// Make `call_batch` return `n` fewer responses than requests, violating the
    /// one-response-per-request contract, to exercise the enricher's count guard.
    #[must_use]
    pub fn with_dropped_responses(mut self, n: usize) -> Self {
        self.dropped = n;
        self
    }
}

#[async_trait::async_trait]
impl Backend for MockBackend {
    type Request<'a> = SttRequest<'a>;
    type Response = SttResponse;

    fn provenance(&self) -> ModelEvent {
        ModelEvent {
            name: "mock-stt".into(),
            ..ModelEvent::default()
        }
    }

    async fn call(&self, _request: SttRequest<'_>) -> Result<SttResponse> {
        Ok(SttResponse::new(self.segments.clone()))
    }

    async fn call_batch(&self, requests: Vec<SttRequest<'_>>) -> Result<Vec<SttResponse>> {
        let kept = requests.len().saturating_sub(self.dropped);
        Ok((0..kept)
            .map(|_| SttResponse::new(self.segments.clone()))
            .collect())
    }
}
