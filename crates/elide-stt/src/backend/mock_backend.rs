//! [`MockBackend`]: stand-in [`SttBackend`] for tests, examples, and as a
//! default before a real backend is configured.

use elide_core::Result;
use elide_core::entity::audit::ModelEvent;
use elide_core::modality::audio::TranscriptSegment;

use super::{SttBackend, SttRequest, SttResponse};

/// Mock STT backend: returns a fixed set of segments on every call.
///
/// Empty by default ([`new`](Self::new) / [`default`](Default::default)), the
/// no-op stub examples and offline wiring rely on, transcribing nothing. Give it
/// canned segments with [`with`](Self::with) to have every call enrich with the
/// same [`Transcription`], for tests that need a real artifact to read back.
///
/// [`Transcription`]: elide_core::modality::audio::Transcription
#[derive(Debug, Default, Clone)]
pub struct MockBackend {
    segments: Vec<TranscriptSegment>,
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
    /// [`Transcription`]: elide_core::modality::audio::Transcription
    #[must_use]
    pub fn with(segments: Vec<TranscriptSegment>) -> Self {
        Self { segments }
    }
}

#[async_trait::async_trait]
impl SttBackend for MockBackend {
    fn provenance(&self) -> ModelEvent {
        ModelEvent {
            name: "mock-stt".into(),
            ..ModelEvent::default()
        }
    }

    async fn transcribe(&self, _request: SttRequest<'_>) -> Result<SttResponse> {
        Ok(SttResponse::new(self.segments.clone()))
    }
}
