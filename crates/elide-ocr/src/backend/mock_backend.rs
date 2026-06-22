//! [`MockBackend`]: stand-in [`OcrBackend`] for tests, examples, and as a
//! default before a real backend is configured.

use elide_core::Result;
use elide_core::entity::provenance::ModelEvent;

use super::{OcrBackend, OcrRequest, OcrResponse};

/// Mock OCR backend: every call returns an empty response.
///
/// Useful as a test stub, in examples that must run without a model, and
/// as the default OCR backend when the operator wants the enricher wired
/// but isn't ready to configure a real backend.
#[derive(Debug, Default, Clone, Copy)]
pub struct MockBackend;

#[async_trait::async_trait]
impl OcrBackend for MockBackend {
    fn provenance(&self) -> ModelEvent {
        ModelEvent {
            name: "mock-ocr".into(),
            ..ModelEvent::default()
        }
    }

    async fn recognize(&self, _request: OcrRequest<'_>) -> Result<OcrResponse> {
        Ok(OcrResponse::default())
    }
}
