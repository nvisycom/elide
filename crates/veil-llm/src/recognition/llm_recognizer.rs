//! [`LlmRecognizer`]: LLM-driven recognizer.
//!
//! Generic over [`Modality`] so one type drives text and image
//! detection through the same surface. Holds an
//! `Arc<dyn LlmBackend>` for the swappable LLM plumbing plus an
//! `Arc<dyn Prompt<M>>` for the swappable modality-specific
//! prompt-build + response-lift. The recognizer body is
//! modality-agnostic: build prompt, send to backend, lift reply.

use std::sync::Arc;

use derive_builder::Builder;
use veil_core::modality::Modality;
use veil_core::recognition::{Recognizer, RecognizerId, RecognizerInput, RecognizerOutput};
use veil_core::{Error, Result};

use super::prompt::Prompt;
use crate::backend::{LlmBackend, LlmRequest};

/// LLM-driven recognizer.
#[derive(Clone, Builder)]
#[builder(
    name = "LlmRecognizerBuilder",
    pattern = "owned",
    setter(into, prefix = "with"),
    build_fn(error = "Error", name = "try_build", private)
)]
pub struct LlmRecognizer<M: Modality> {
    /// Recognizer name. Surfaced in the recognition event on every
    /// emitted entity and used as the recognizer id.
    name: String,
    /// Backend that sends the prompt to the model and returns its
    /// reply. Required. Set via [`with_backend`], which accepts any
    /// concrete [`LlmBackend`] impl by value and wraps it in `Arc`
    /// internally.
    ///
    /// [`with_backend`]: LlmRecognizerBuilder::with_backend
    #[builder(setter(custom))]
    backend: Arc<dyn LlmBackend>,
    /// Modality-specific prompt builder + response lifter. Required.
    /// Set via [`with_prompt`], which accepts any concrete
    /// [`Prompt<M>`] impl by value and wraps it in `Arc` internally.
    ///
    /// [`with_prompt`]: LlmRecognizerBuilder::with_prompt
    #[builder(setter(custom))]
    prompt: Arc<dyn Prompt<M>>,
}

impl<M: Modality> LlmRecognizer<M> {
    /// Start the chainable builder. `name`, `backend`, and `prompt`
    /// are required; calling [`build`] without them returns a
    /// validation error.
    ///
    /// [`build`]: LlmRecognizerBuilder::build
    #[must_use]
    pub fn builder() -> LlmRecognizerBuilder<M> {
        LlmRecognizerBuilder::default()
    }

    /// Recognizer name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Borrow the configured backend.
    #[must_use]
    pub fn backend(&self) -> &Arc<dyn LlmBackend> {
        &self.backend
    }

    /// Borrow the configured prompt.
    #[must_use]
    pub fn prompt(&self) -> &Arc<dyn Prompt<M>> {
        &self.prompt
    }
}

impl<M: Modality> LlmRecognizerBuilder<M> {
    /// Set the [`LlmBackend`] that powers this recognizer. Accepts
    /// any concrete impl by value and wraps it in `Arc`. Required:
    /// `build` errors when this hasn't been called.
    #[must_use]
    pub fn with_backend<B: LlmBackend>(mut self, backend: B) -> Self {
        self.backend = Some(Arc::new(backend));
        self
    }

    /// Set the modality-specific [`Prompt`] strategy. Accepts any
    /// concrete impl by value and wraps it in `Arc`. Required:
    /// `build` errors when this hasn't been called.
    #[must_use]
    pub fn with_prompt<P: Prompt<M>>(mut self, prompt: P) -> Self {
        self.prompt = Some(Arc::new(prompt));
        self
    }

    /// Finish the builder. Errors when `name`, `backend`, or
    /// `prompt` is unset.
    pub fn build(self) -> Result<LlmRecognizer<M>> {
        self.try_build()
    }
}

impl<M: Modality> Recognizer<M> for LlmRecognizer<M> {
    fn id(&self) -> RecognizerId {
        RecognizerId::new(self.name.clone(), env!("CARGO_PKG_VERSION"))
    }

    async fn recognize(&self, input: &RecognizerInput<M>) -> Result<RecognizerOutput<M>> {
        let prompt = self.prompt.build(input);
        let request = LlmRequest {
            prompt: &prompt,
            schema: self.prompt.schema(),
        };
        let response = self.backend.predict(request).await?;
        let entities = self.prompt.lift(&response, input);
        Ok(RecognizerOutput::new(entities))
    }
}
