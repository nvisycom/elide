//! Backend layer: the [`LlmBackend<M>`] trait and its shipped impls.
//!
//! A backend turns a rendered prompt into the model's structured candidate
//! batch. It is generic over the modality `M`: it extracts a
//! [`Candidates<M::Item>`] — the typed candidate batch the model is asked
//! to produce. A backend declares which modalities it serves by which
//! `LlmBackend<M>` impls it carries. Prompt wording lives in
//! [`crate::prompt`]; localizing candidates into entities lives in the
//! recognizer (via [`LlmModality::lift`]).
//!
//! [`Candidates<M::Item>`]: crate::candidates::Candidates
//! [`LlmModality::lift`]: crate::backend::LlmModality::lift

#[cfg(feature = "rig")]
mod http;
mod llm_request;
mod llm_response;
#[cfg(any(test, feature = "mock"))]
mod mock_backend;
#[cfg(feature = "rig")]
mod rig;

use elide_core::Result;

pub use self::llm_request::LlmRequest;
pub use self::llm_response::LlmResponse;
#[cfg(any(test, feature = "mock"))]
#[cfg_attr(docsrs, doc(cfg(feature = "mock")))]
pub use self::mock_backend::MockBackend;
#[cfg(feature = "rig")]
#[cfg_attr(docsrs, doc(cfg(feature = "rig")))]
pub use self::rig::{RigBackend, RigConfig};
pub use crate::modality::LlmModality;

/// Per-call LLM backend for modality `M`.
///
/// Implemented by everything that turns a rendered prompt into the model's
/// structured candidate batch: rig-backed providers (OpenAI, Anthropic,
/// Gemini, Ollama) and the in-process no-op test stub.
///
/// Object-safe: recognizers hold `Arc<dyn LlmBackend<M>>` and dispatch per
/// call. The candidate type is fixed by `M`, so there is no free generic
/// on the call.
#[async_trait::async_trait]
pub trait LlmBackend<M: LlmModality>: Send + Sync + 'static {
    /// Send `request` to the model and return its structured candidate
    /// batch.
    ///
    /// The prompt wording is rendered by the recognizer's
    /// [`Prompt`]; the backend folds in the source
    /// payload (e.g. image bytes) to build the provider message, and
    /// constrains the model to produce the candidate shape for `M`.
    ///
    /// # Errors
    ///
    /// Returns the underlying transport / provider / extraction error.
    ///
    /// [`Prompt`]: crate::prompt::Prompt
    async fn extract(&self, request: LlmRequest<'_, M>) -> Result<LlmResponse<M>>;

    /// Model name the backend is configured to call. Recognizers stamp
    /// this into entity trail provenance so post-hoc analysis can
    /// attribute scores to a specific model.
    fn model(&self) -> &str;
}
