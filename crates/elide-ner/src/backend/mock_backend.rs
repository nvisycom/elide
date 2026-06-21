//! [`MockBackend`]: stand-in [`NerBackend`] for tests, examples, and as a
//! default before a real backend is configured.

use elide_core::Result;
use elide_core::entity::provenance::ModelEvent;

use super::{NerBackend, NerRequest, NerResponse};

/// Mock NER backend: every call returns an empty response.
///
/// Useful as a test stub, in examples that must run without a model, and
/// as the default NER backend when the operator wants the recognizer
/// wired but isn't ready to configure a real backend.
#[derive(Debug, Default, Clone, Copy)]
pub struct MockBackend;

#[async_trait::async_trait]
impl NerBackend for MockBackend {
    fn provenance(&self) -> ModelEvent {
        ModelEvent {
            name: "mock-ner".into(),
            ..ModelEvent::default()
        }
    }

    async fn recognize(&self, _request: NerRequest<'_>) -> Result<NerResponse> {
        Ok(NerResponse::default())
    }
}
