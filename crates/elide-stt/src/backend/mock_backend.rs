//! [`MockBackend`]: stand-in [`SttBackend`] for tests, examples, and as a
//! default before a real backend is configured.

use elide_core::Result;
use elide_core::entity::provenance::ModelEvent;

use super::{SttBackend, SttRequest, SttResponse};

/// Mock STT backend: every call returns an empty response.
///
/// Useful as a test stub, in examples that must run without a model, and
/// as the default STT backend when the operator wants the extractor wired
/// but isn't ready to configure a real backend.
#[derive(Debug, Default, Clone, Copy)]
pub struct MockBackend;

#[async_trait::async_trait]
impl SttBackend for MockBackend {
    fn provenance(&self) -> ModelEvent {
        ModelEvent {
            name: "mock-stt".into(),
            ..ModelEvent::default()
        }
    }

    async fn transcribe(&self, _request: SttRequest<'_>) -> Result<SttResponse> {
        Ok(SttResponse::default())
    }
}
