//! [`LlmRecognizer`]: LLM-driven recognizer.
//!
//! Generic over its LLM backend `B`, whose [`Modality`](LlmBackend::Modality)
//! it recognizes for, so one type drives text and image detection through the
//! same surface. Holds `B` for the swappable LLM plumbing plus an
//! `Arc<dyn Prompt<B::Modality>>` for the swappable prompt wording. The
//! recognizer renders the prompt, asks the backend to extract the candidate
//! batch, then lifts each candidate into an entity via [`LlmModality::lift`],
//! attributed to the model the backend reports.

use std::sync::Arc;

use elide_core::Result;
use elide_core::backend::BackendBatchExt;
use elide_core::entity::Entity;
use elide_core::primitive::ComponentId;
use elide_core::recognition::{Context, Recognizer, Subject};
use hipstr::HipStr;

#[cfg(any(test, feature = "mocks"))]
use crate::backend::MockBackend;
use crate::backend::{LlmBackend, LlmRequest};
use crate::modality::LlmModality;
use crate::prompt::{DefaultPrompt, Prompt};

const TARGET: &str = "elide_llm::recognition";

/// A recognizer that prompts an LLM, over an [`LlmBackend`] `B`.
///
/// The backend fixes both the plumbing and the modality: `B::Modality` is what
/// the recognizer recognizes for, so there is no separate modality type
/// parameter. Built with [`new`](Self::new) (an explicit [`Prompt`]) or
/// [`with_default`](Self::with_default) (the built-in [`DefaultPrompt`]).
#[derive(Clone)]
pub struct LlmRecognizer<B: LlmBackend> {
    /// Recognizer name, its id; the crate name unless set.
    name: HipStr<'static>,
    /// Backend that sends the prompt to the model and returns the structured
    /// candidate batch. May be a decorated backend.
    backend: B,
    /// Modality-specific prompt wording.
    prompt: Arc<dyn Prompt<B::Modality>>,
}

impl<B: LlmBackend> LlmRecognizer<B> {
    /// An LLM recognizer over `backend` using `prompt`, named after the crate
    /// until [`with_name`](Self::with_name) sets another.
    #[must_use]
    pub fn new<P: Prompt<B::Modality>>(backend: B, prompt: P) -> Self {
        Self {
            name: HipStr::borrowed(env!("CARGO_PKG_NAME")),
            backend,
            prompt: Arc::new(prompt),
        }
    }

    /// An LLM recognizer over `backend` with its modality's [`DefaultPrompt`].
    ///
    /// [`DefaultPrompt`]: crate::prompt::DefaultPrompt
    #[must_use]
    pub fn with_default(backend: B) -> Self
    where
        DefaultPrompt: Prompt<B::Modality>,
    {
        Self::new(backend, DefaultPrompt)
    }

    /// Set the recognizer name, surfaced as its id; set one to tell several
    /// LLM recognizers apart.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<HipStr<'static>>) -> Self {
        self.name = name.into();
        self
    }

    /// Recognizer name: the one set with [`with_name`](Self::with_name), or the
    /// crate name.
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
    pub fn prompt(&self) -> &Arc<dyn Prompt<B::Modality>> {
        &self.prompt
    }
}

#[cfg(any(test, feature = "mocks"))]
impl<M> LlmRecognizer<MockBackend<M>>
where
    M: LlmModality,
    MockBackend<M>: LlmBackend<Modality = M>,
    DefaultPrompt: Prompt<M>,
{
    /// An LLM recognizer over the no-op [`MockBackend`] and the built-in prompt.
    /// Produces no entities.
    ///
    /// The modality is fixed only by the backend type, so name it at the call
    /// site — `LlmRecognizer::<MockBackend<Text>>::mock()` — unless it is
    /// inferable from use.
    ///
    /// [`MockBackend`]: crate::backend::MockBackend
    #[cfg_attr(docsrs, doc(cfg(feature = "mocks")))]
    #[must_use]
    pub fn mock() -> Self {
        Self::with_default(MockBackend::new())
    }
}

#[async_trait::async_trait]
impl<B: LlmBackend> Recognizer<B::Modality> for LlmRecognizer<B> {
    fn id(&self) -> ComponentId {
        ComponentId::new(self.name.clone(), env!("CARGO_PKG_VERSION"))
    }

    async fn recognize(
        &self,
        subject: &Subject<B::Modality>,
        ctx: &Context<'_>,
    ) -> Result<Vec<Entity<B::Modality>>> {
        let prompt = self.prompt.build(subject, ctx)?;
        let response = self.backend.call(request(&prompt, subject, ctx)).await?;
        let model = self.backend.provenance();
        let candidates = response.candidates.entities.len();
        let entities = B::Modality::lift(response.candidates, subject.data(), &model);
        tracing::debug!(
            target: TARGET,
            recognizer = %self.name,
            model = %model.name,
            candidates,
            entities = entities.len(),
            "llm candidates lifted",
        );
        Ok(entities)
    }

    /// One prompt per subject, sent as one batch through the backend's
    /// [`call_batch`](elide_core::backend::Backend::call_batch).
    async fn recognize_batch(
        &self,
        subjects: &[&Subject<B::Modality>],
        ctx: &Context<'_>,
    ) -> Result<Vec<Vec<Entity<B::Modality>>>> {
        let prompts = subjects
            .iter()
            .map(|subject| self.prompt.build(subject, ctx))
            .collect::<Result<Vec<_>>>()?;
        let requests = prompts
            .iter()
            .zip(subjects)
            .map(|(prompt, subject)| request(prompt, subject, ctx))
            .collect();
        let responses = self.backend.call_aligned(requests).await?;
        let model = self.backend.provenance();
        let candidates: usize = responses.iter().map(|r| r.candidates.entities.len()).sum();
        let lifted: Vec<_> = responses
            .into_iter()
            .zip(subjects)
            .map(|(response, subject)| {
                B::Modality::lift(response.candidates, subject.data(), &model)
            })
            .collect();
        tracing::debug!(
            target: TARGET,
            recognizer = %self.name,
            model = %model.name,
            subjects = subjects.len(),
            candidates,
            entities = lifted.iter().map(Vec::len).sum::<usize>(),
            "llm candidates lifted",
        );
        Ok(lifted)
    }
}

/// The backend request for `subject`, carrying its rendered `prompt` and the
/// call's primary language.
fn request<'a, M: LlmModality>(
    prompt: &'a str,
    subject: &'a Subject<M>,
    ctx: &'a Context<'_>,
) -> LlmRequest<'a, M> {
    LlmRequest {
        prompt,
        data: subject.data(),
        language: ctx.languages(subject).primary(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use elide_core::backend::Backend;
    use elide_core::entity::audit::{AuditKind, ModelId};
    use elide_core::modality::text::{Text, TextData};
    use elide_core::recognition::Scope;

    use super::*;
    use crate::backend::LlmResponse;
    use crate::candidates::{Candidates, TextCandidate};

    /// Finds "Jane" in every text, counting its single and batch calls.
    #[derive(Clone, Default)]
    struct Jane {
        calls: Arc<AtomicUsize>,
        batches: Arc<AtomicUsize>,
    }

    fn jane() -> LlmResponse<Text> {
        LlmResponse::new(Candidates {
            entities: vec![TextCandidate {
                value: "Jane".to_owned(),
                description: None,
                label: "person_name".to_owned(),
                confidence: Some(0.9),
                context: Some("Jane".to_owned()),
                coreference: None,
            }],
        })
    }

    #[async_trait::async_trait]
    impl Backend for Jane {
        type Request<'a> = LlmRequest<'a, Text>;
        type Response = LlmResponse<Text>;

        fn provenance(&self) -> ModelId {
            ModelId {
                name: "jane-finder".into(),
                ..ModelId::default()
            }
        }

        async fn call(&self, _request: LlmRequest<'_, Text>) -> Result<LlmResponse<Text>> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(jane())
        }

        async fn call_batch(
            &self,
            requests: Vec<LlmRequest<'_, Text>>,
        ) -> Result<Vec<LlmResponse<Text>>> {
            self.batches.fetch_add(1, Ordering::Relaxed);
            Ok(requests.iter().map(|_| jane()).collect())
        }
    }

    #[tokio::test]
    async fn entities_name_the_backend_model() {
        let recognizer = LlmRecognizer::with_default(Jane::default());
        let scope = Scope::new();
        let subject = Subject::<Text>::new(TextData::new("Jane"));

        let out = recognizer
            .recognize(&subject, &Context::new(&scope))
            .await
            .unwrap();

        let AuditKind::Model(event) = &out[0].audit.events()[0].kind else {
            panic!("a model event");
        };
        assert_eq!(event.model.name, "jane-finder");
    }

    #[tokio::test]
    async fn recognize_batch_sends_one_batch() {
        let backend = Jane::default();
        let counters = backend.clone();
        let recognizer = LlmRecognizer::with_default(backend);
        let scope = Scope::new();
        let subjects: [Subject<Text>; _] = [
            Subject::new(TextData::new("Jane")),
            Subject::new(TextData::new("Jane")),
        ];

        let out = recognizer
            .recognize_batch(&subjects.iter().collect::<Vec<_>>(), &Context::new(&scope))
            .await
            .unwrap();

        assert_eq!(counters.batches.load(Ordering::Relaxed), 1);
        assert_eq!(counters.calls.load(Ordering::Relaxed), 0);
        assert!(out.iter().all(|entities| entities.len() == 1));
    }
}
