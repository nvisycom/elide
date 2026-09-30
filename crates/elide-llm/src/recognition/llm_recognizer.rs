//! [`LlmRecognizer`]: LLM-driven recognizer.
//!
//! Generic over [`LlmModality`] so one type drives text and image
//! detection through the same surface. Holds an `Arc<dyn LlmBackend<M>>`
//! for the swappable LLM plumbing plus an `Arc<dyn Prompt<M>>` for the
//! swappable prompt wording. The recognizer renders the prompt, asks the
//! backend to extract the candidate batch, then lifts each candidate into
//! an entity via [`LlmModality::lift`].

use std::marker::PhantomData;
use std::sync::Arc;

use elide_core::primitive::ComponentId;
use elide_core::recognition::{Recognition, Recognizer, RecognizerContext, Subject};
use elide_core::{Error, ErrorKind, Result};

#[cfg(any(test, feature = "mocks"))]
use crate::backend::MockBackend;
use crate::backend::{LlmBackend, LlmRequest};
use crate::modality::LlmModality;
use crate::prompt::{DefaultPrompt, Prompt};

/// LLM-driven recognizer, generic over its modality `M` and [`LlmBackend`] `B`.
///
/// `B` defaults to `()` (no backend), so `LlmRecognizer::<M>::builder()` names
/// only the modality; [`with_backend`](LlmRecognizerBuilder::with_backend) fixes
/// the backend type.
#[derive(Clone)]
pub struct LlmRecognizer<M: LlmModality, B = ()> {
    /// Recognizer name. Surfaced in the recognition event on every emitted
    /// entity and used as the recognizer id.
    name: String,
    /// Backend that sends the prompt to the model and returns the structured
    /// candidate batch. May be a `Metered`
    /// wrapper.
    backend: B,
    /// Modality-specific prompt wording.
    prompt: Arc<dyn Prompt<M>>,
}

impl<M: LlmModality> LlmRecognizer<M, ()> {
    /// Start the chainable builder. `name`, `backend`, and `prompt` are required.
    #[must_use]
    pub fn builder() -> LlmRecognizerBuilder<M, ()> {
        LlmRecognizerBuilder::default()
    }
}

impl<M: LlmModality, B> LlmRecognizer<M, B> {
    /// Recognizer name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Borrow the configured backend.
    #[must_use]
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Borrow the configured prompt.
    #[must_use]
    pub fn prompt(&self) -> &Arc<dyn Prompt<M>> {
        &self.prompt
    }

    fn recognizer_id(&self) -> ComponentId {
        ComponentId::new(self.name.clone(), env!("CARGO_PKG_VERSION"))
    }
}

/// Chainable builder for an [`LlmRecognizer`], generic over the backend `B` that
/// [`with_backend`](Self::with_backend) sets.
pub struct LlmRecognizerBuilder<M: LlmModality, B = ()> {
    name: Option<String>,
    backend: Option<B>,
    prompt: Option<Arc<dyn Prompt<M>>>,
    _modality: PhantomData<fn() -> M>,
}

impl<M: LlmModality> Default for LlmRecognizerBuilder<M, ()> {
    fn default() -> Self {
        Self {
            name: None,
            backend: None,
            prompt: None,
            _modality: PhantomData,
        }
    }
}

impl<M: LlmModality, B> LlmRecognizerBuilder<M, B> {
    /// Set the recognizer name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the [`LlmBackend`] that powers this recognizer, fixing the builder's
    /// backend type. Required.
    #[must_use]
    pub fn with_backend<B2: LlmBackend<M>>(self, backend: B2) -> LlmRecognizerBuilder<M, B2> {
        LlmRecognizerBuilder {
            name: self.name,
            backend: Some(backend),
            prompt: self.prompt,
            _modality: PhantomData,
        }
    }

    /// Set the modality-specific [`Prompt`] wording. Required.
    #[must_use]
    pub fn with_prompt<P: Prompt<M>>(mut self, prompt: P) -> Self {
        self.prompt = Some(Arc::new(prompt));
        self
    }

    /// Use the built-in [`DefaultPrompt`] for this modality.
    ///
    /// [`DefaultPrompt`]: crate::prompt::DefaultPrompt
    #[must_use]
    pub fn with_default_prompt(self) -> Self
    where
        DefaultPrompt: Prompt<M>,
    {
        self.with_prompt(DefaultPrompt)
    }
}

impl<M: LlmModality> LlmRecognizerBuilder<M, ()> {
    /// Wire the no-op [`MockBackend`] as this recognizer's backend.
    ///
    /// [`MockBackend`]: crate::backend::MockBackend
    #[cfg(any(test, feature = "mocks"))]
    #[cfg_attr(docsrs, doc(cfg(feature = "mocks")))]
    #[must_use]
    pub fn with_mock_backend(self) -> LlmRecognizerBuilder<M, MockBackend<M>>
    where
        MockBackend<M>: LlmBackend<M>,
    {
        self.with_backend(MockBackend::new())
    }
}

impl<M: LlmModality, B: LlmBackend<M>> LlmRecognizerBuilder<M, B> {
    /// Finish the builder. Errors when `name`, `backend`, or `prompt` is unset.
    pub fn build(self) -> Result<LlmRecognizer<M, B>> {
        Ok(LlmRecognizer {
            name: self.name.ok_or_else(|| {
                Error::new(ErrorKind::Configuration, "LlmRecognizer requires a name")
            })?,
            backend: self.backend.ok_or_else(|| {
                Error::new(ErrorKind::Configuration, "LlmRecognizer requires a backend")
            })?,
            prompt: self.prompt.ok_or_else(|| {
                Error::new(ErrorKind::Configuration, "LlmRecognizer requires a prompt")
            })?,
        })
    }
}

#[async_trait::async_trait]
impl<M: LlmModality, B: LlmBackend<M>> Recognizer<M> for LlmRecognizer<M, B> {
    fn id(&self) -> ComponentId {
        self.recognizer_id()
    }

    async fn recognize(
        &self,
        subject: &Subject<M>,
        ctx: &RecognizerContext<'_, M>,
    ) -> Result<Recognition<M>> {
        let prompt = self.prompt.build(subject, ctx);
        let response = self
            .backend
            .call(LlmRequest::new(&prompt, subject.data()))
            .await?;
        let entities = M::lift(response.candidates, subject.data());
        Ok(Recognition::new(entities))
    }
}
