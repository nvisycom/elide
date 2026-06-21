//! [`LlmRecognizer`]: LLM-driven recognizer.
//!
//! Generic over [`LlmModality`] so one type drives text and image
//! detection through the same surface. Holds an `Arc<dyn LlmBackend<M>>`
//! for the swappable LLM plumbing plus an `Arc<dyn Prompt<M>>` for the
//! swappable prompt wording. The recognizer renders the prompt, asks the
//! backend to extract the candidate batch, then lifts each candidate into
//! an entity via [`LlmModality::lift`].

use std::sync::Arc;

use derive_builder::Builder;
use elide_core::entity::Entity;
use elide_core::recognition::{Recognizer, RecognizerContext, RecognizerId};
use elide_core::{Error, Result};

use crate::backend::{LlmBackend, LlmRequest};
use crate::modality::LlmModality;
use crate::prompt::Prompt;

/// LLM-driven recognizer.
#[derive(Clone, Builder)]
#[builder(
    name = "LlmRecognizerBuilder",
    pattern = "owned",
    setter(into, prefix = "with"),
    build_fn(error = "Error", name = "try_build", private)
)]
pub struct LlmRecognizer<M: LlmModality> {
    /// Recognizer name. Surfaced in the recognition event on every
    /// emitted entity and used as the recognizer id.
    name: String,
    /// Backend that sends the prompt to the model and returns the
    /// structured candidate batch. Required. Set via [`with_backend`].
    ///
    /// [`with_backend`]: LlmRecognizerBuilder::with_backend
    #[builder(setter(custom))]
    backend: Arc<dyn LlmBackend<M>>,
    /// Modality-specific prompt wording. Required. Set via
    /// [`with_prompt`].
    ///
    /// [`with_prompt`]: LlmRecognizerBuilder::with_prompt
    #[builder(setter(custom))]
    prompt: Arc<dyn Prompt<M>>,
}

impl<M: LlmModality> LlmRecognizer<M> {
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
    pub fn backend(&self) -> &Arc<dyn LlmBackend<M>> {
        &self.backend
    }

    /// Borrow the configured prompt.
    #[must_use]
    pub fn prompt(&self) -> &Arc<dyn Prompt<M>> {
        &self.prompt
    }

    fn recognizer_id(&self) -> RecognizerId {
        RecognizerId::new(self.name.clone(), env!("CARGO_PKG_VERSION"))
    }
}

impl<M: LlmModality> LlmRecognizerBuilder<M> {
    /// Set the [`LlmBackend`] that powers this recognizer. Accepts
    /// any concrete impl by value and wraps it in `Arc`. Required:
    /// `build` errors when this hasn't been called.
    #[must_use]
    pub fn with_backend<B: LlmBackend<M>>(mut self, backend: B) -> Self {
        self.backend = Some(Arc::new(backend));
        self
    }

    /// Wire the no-op [`MockBackend`] as this recognizer's backend.
    ///
    /// Convenience for tests, examples, and offline wiring: the
    /// recognizer is fully built but produces no entities. Equivalent to
    /// `with_backend(MockBackend)`.
    ///
    /// [`MockBackend`]: crate::backend::MockBackend
    #[cfg(any(test, feature = "mock"))]
    #[cfg_attr(docsrs, doc(cfg(feature = "mock")))]
    #[must_use]
    pub fn with_mock_backend(self) -> Self
    where
        crate::backend::MockBackend: LlmBackend<M>,
    {
        self.with_backend(crate::backend::MockBackend)
    }

    /// Set the modality-specific [`Prompt`] wording. Accepts any concrete
    /// impl by value and wraps it in `Arc`. Required: `build` errors when
    /// this hasn't been called.
    #[must_use]
    pub fn with_prompt<P: Prompt<M>>(mut self, prompt: P) -> Self {
        self.prompt = Some(Arc::new(prompt));
        self
    }

    /// Use the built-in [`DefaultPrompt`] for this modality.
    ///
    /// Convenience for the common case: equivalent to
    /// `with_prompt(DefaultPrompt)`.
    ///
    /// [`DefaultPrompt`]: crate::prompt::DefaultPrompt
    #[must_use]
    pub fn with_default_prompt(self) -> Self
    where
        crate::prompt::DefaultPrompt: Prompt<M>,
    {
        self.with_prompt(crate::prompt::DefaultPrompt)
    }

    /// Finish the builder. Errors when `name`, `backend`, or
    /// `prompt` is unset.
    pub fn build(self) -> Result<LlmRecognizer<M>> {
        self.try_build()
    }
}

impl<M: LlmModality> Recognizer<M> for LlmRecognizer<M> {
    fn id(&self) -> RecognizerId {
        self.recognizer_id()
    }

    async fn recognize(
        &self,
        data: &M::Data,
        ctx: &RecognizerContext<'_, M>,
    ) -> Result<Vec<Entity<M>>> {
        let prompt = self.prompt.build(data, ctx);
        let response = self.backend.extract(LlmRequest::new(&prompt, data)).await?;
        Ok(M::lift(response.candidates, data))
    }
}
