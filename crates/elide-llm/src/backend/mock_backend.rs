//! [`MockBackend`]: a no-op [`LlmBackend`] for tests, examples, and as a
//! default before a real provider is configured.

use std::marker::PhantomData;

use elide_core::Result;
use elide_core::backend::Backend;
use elide_core::entity::audit::ModelEvent;

use super::{LlmRequest, LlmResponse};
use crate::modality::LlmModality;

/// An LLM backend that calls no model and returns an empty batch, for modality
/// `M`.
///
/// Every recognizer driven by this backend produces zero entities: the
/// candidate batch is empty. Useful for wiring a pipeline together before a
/// real provider is available, and for tests and examples that must run
/// without network access or credentials.
pub struct MockBackend<M>(PhantomData<fn() -> M>);

impl<M> Default for MockBackend<M> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<M> MockBackend<M> {
    /// A mock backend for modality `M`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl<M: LlmModality> Backend for MockBackend<M> {
    type Request<'a> = LlmRequest<'a, M>;
    type Response = LlmResponse<M>;

    fn provenance(&self) -> ModelEvent {
        ModelEvent {
            name: "mock".into(),
            ..ModelEvent::default()
        }
    }

    async fn call(&self, _request: LlmRequest<'_, M>) -> Result<LlmResponse<M>> {
        Ok(LlmResponse::default())
    }
}
