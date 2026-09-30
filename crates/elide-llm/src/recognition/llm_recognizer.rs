//! [`LlmRecognizer`]: LLM-driven recognizer.
//!
//! Generic over [`LlmModality`] so one type drives text and image
//! detection through the same surface. Holds its LLM backend `B` for the
//! swappable LLM plumbing plus an `Arc<dyn Prompt<M>>` for the
//! swappable prompt wording. The recognizer renders the prompt, asks the
//! backend to extract the candidate batch, then lifts each candidate into
//! an entity via [`LlmModality::lift`].

use std::sync::Arc;

use elide_core::Result;
use elide_core::backend::Backend;
use elide_core::primitive::ComponentId;
use elide_core::recognition::{Recognition, Recognizer, RecognizerContext, Subject};

#[cfg(any(test, feature = "mocks"))]
use crate::backend::MockBackend;
use crate::backend::{LlmRequest, LlmResponse};
use crate::modality::LlmModality;
use crate::prompt::{DefaultPrompt, Prompt};

/// LLM-driven recognizer, generic over its modality `M` and its LLM backend `B` —
/// any [`Backend`] whose request is [`LlmRequest<'_, M>`](LlmRequest) and whose
/// response is [`LlmResponse<M>`](LlmResponse).
///
/// Built with [`new`](Self::new) (an explicit [`Prompt`]) or
/// [`with_default`](Self::with_default) (the built-in [`DefaultPrompt`]); the
/// backend fixes `B`. The prompt is required, so it is a constructor argument
/// rather than a fallible setter.
#[derive(Clone)]
pub struct LlmRecognizer<M: LlmModality, B = ()> {
    /// Optional recognizer name, surfaced in the recognition event on every
    /// emitted entity and used as the recognizer id. `None` falls back to the
    /// crate name; set one to tell several LLM recognizers apart.
    name: Option<String>,
    /// Backend that sends the prompt to the model and returns the structured
    /// candidate batch. May be a `Metered`
    /// wrapper.
    backend: B,
    /// Modality-specific prompt wording.
    prompt: Arc<dyn Prompt<M>>,
}

impl<M, B> LlmRecognizer<M, B>
where
    M: LlmModality,
    B: for<'a> Backend<Request<'a> = LlmRequest<'a, M>, Response = LlmResponse<M>>,
{
    /// An LLM recognizer over `backend` using `prompt`.
    ///
    /// Unnamed by default (its id falls back to the crate name); set a name with
    /// [`with_name`](Self::with_name) when running more than one.
    #[must_use]
    pub fn new<P: Prompt<M>>(backend: B, prompt: P) -> Self {
        Self {
            name: None,
            backend,
            prompt: Arc::new(prompt),
        }
    }

    /// An LLM recognizer over `backend` using the built-in [`DefaultPrompt`] for
    /// this modality.
    ///
    /// [`DefaultPrompt`]: crate::prompt::DefaultPrompt
    #[must_use]
    pub fn with_default(backend: B) -> Self
    where
        DefaultPrompt: Prompt<M>,
    {
        Self::new(backend, DefaultPrompt)
    }

    /// Set the recognizer name, surfaced as its id.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

#[cfg(any(test, feature = "mocks"))]
impl<M> LlmRecognizer<M, MockBackend<M>>
where
    M: LlmModality,
    MockBackend<M>: for<'a> Backend<Request<'a> = LlmRequest<'a, M>, Response = LlmResponse<M>>,
    DefaultPrompt: Prompt<M>,
{
    /// An LLM recognizer over the no-op [`MockBackend`] and the built-in prompt.
    ///
    /// [`MockBackend`]: crate::backend::MockBackend
    #[cfg_attr(docsrs, doc(cfg(feature = "mocks")))]
    #[must_use]
    pub fn mock() -> Self {
        Self::with_default(MockBackend::new())
    }
}

impl<M: LlmModality, B> LlmRecognizer<M, B> {
    /// Recognizer name — the one set with [`with_name`](Self::with_name), or the
    /// crate name when unset.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_deref().unwrap_or(env!("CARGO_PKG_NAME"))
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
        ComponentId::new(self.name().to_owned(), env!("CARGO_PKG_VERSION"))
    }
}

#[async_trait::async_trait]
impl<M, B> Recognizer<M> for LlmRecognizer<M, B>
where
    M: LlmModality,
    B: for<'a> Backend<Request<'a> = LlmRequest<'a, M>, Response = LlmResponse<M>>,
{
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
