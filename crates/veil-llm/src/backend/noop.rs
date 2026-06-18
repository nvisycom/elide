//! [`NoopBackend`]: a no-op [`LlmBackend`] for tests, examples, and as a
//! default before a real provider is configured.

use veil_core::Result;

use super::{LlmBackend, LlmRequest, LlmResponse};

/// An [`LlmBackend`] that calls no model and returns an empty reply.
///
/// Every recognizer driven by this backend produces zero entities: the
/// empty response parses to an empty candidate set. Useful for wiring a
/// pipeline together before a real provider is available, and for tests
/// and examples that must run without network access or credentials.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopBackend;

#[async_trait::async_trait]
impl LlmBackend for NoopBackend {
    async fn predict(&self, _request: LlmRequest<'_>) -> Result<LlmResponse> {
        Ok(LlmResponse::default())
    }

    fn model(&self) -> &str {
        "noop"
    }
}
