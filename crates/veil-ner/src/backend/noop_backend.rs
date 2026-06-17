//! [`NoopBackend`]: no-op [`NerBackend`] for tests and as a default
//! NER backend when no real backend is configured yet.

use veil_core::Result;
use veil_core::provenance::ModelEvent;

use super::ner_backend::{NerBackend, NerRequest, NerResponse};

/// No-op NER backend: every call returns an empty response.
///
/// Useful as a test stub and as the default NER backend when the
/// operator wants the NER recognizer wired but isn't ready to configure
/// a real backend.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopBackend;

#[async_trait::async_trait]
impl NerBackend for NoopBackend {
    fn provenance(&self) -> ModelEvent {
        ModelEvent {
            name: "noop-ner".into(),
            ..ModelEvent::default()
        }
    }

    async fn recognize(&self, _request: NerRequest<'_>) -> Result<NerResponse> {
        Ok(NerResponse::default())
    }
}
