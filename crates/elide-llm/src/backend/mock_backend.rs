//! [`MockBackend`]: a no-op [`LlmBackend`] for tests, examples, and as a
//! default before a real provider is configured.

use elide_core::Result;

use super::{LlmBackend, LlmRequest, LlmResponse};
use crate::modality::LlmModality;

/// An [`LlmBackend`] that calls no model and returns an empty batch.
///
/// Every recognizer driven by this backend produces zero entities: the
/// candidate batch is empty. Useful for wiring a pipeline together before a
/// real provider is available, and for tests and examples that must run
/// without network access or credentials.
#[derive(Debug, Clone, Copy, Default)]
pub struct MockBackend;

#[async_trait::async_trait]
impl<M: LlmModality> LlmBackend<M> for MockBackend {
    async fn extract(&self, _request: LlmRequest<'_, M>) -> Result<LlmResponse<M>> {
        Ok(LlmResponse::default())
    }

    fn model(&self) -> &str {
        "mock"
    }
}
