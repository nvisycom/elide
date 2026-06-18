//! Backend layer: the modality-agnostic [`LlmBackend`] trait that
//! turns a prompt + schema into the model's reply, plus its shipped
//! impls.
//!
//! Modality-specific work (prompt construction, response → entity
//! lifting) lives in [`crate::LlmRecognizer`]; backends only handle
//! provider dispatch, structured-output, retries, and usage
//! tracking.

pub mod http;
mod request;
mod response;
pub mod rig;

use veil_core::Result;

pub use self::request::LlmRequest;
pub use self::response::LlmResponse;

/// Per-call LLM backend.
///
/// Implemented by everything that turns a `(prompt, schema)` pair
/// into the model's text reply: rig-backed providers (OpenAI,
/// Anthropic, Gemini, Ollama), externalised inference gateways, the
/// in-process no-op test stub.
///
/// Object-safe: recognizers hold `Arc<dyn LlmBackend>` and dispatch
/// per call.
#[async_trait::async_trait]
pub trait LlmBackend: Send + Sync + 'static {
    /// Send `request` to the model and return its reply.
    ///
    /// # Errors
    ///
    /// Returns the underlying transport / provider / parse error.
    async fn predict(&self, request: LlmRequest<'_>) -> Result<LlmResponse>;

    /// Model name the backend is configured to call. Recognizers
    /// stamp this into entity trail provenance so post-hoc analysis
    /// can attribute scores to a specific model.
    fn model(&self) -> &str;
}
