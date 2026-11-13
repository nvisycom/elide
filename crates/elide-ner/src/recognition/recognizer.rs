//! [`NerRecognizer`]: unified NER recognizer that drives any
//! NER backend.
//!
//! Holds its NER backend `B`. On each `recognize` call it asks the backend for
//! spans of the call's catalog labels (`None` when the catalog is empty, leaving
//! the backend to emit what it natively produces), then emits an entity for
//! every canonical span the backend returns. Restricting them to the catalog is
//! the analyzer's job, after reconciliation. Which labels a model supports,
//! which it ignores, and how its scores scale are properties of the model, so
//! they are backend decorators ([`LabelScoped`], [`LabelExcluded`],
//! [`ScoreScaled`]), not recognizer settings.
//!
//! Implements [`Recognizer<Text>`] so it composes with the
//! rest of the platform through the same trait every other text
//! recognizer uses.
//!
//! [`LabelScoped`]: crate::backend::LabelScoped
//! [`LabelExcluded`]: crate::backend::LabelExcluded
//! [`ScoreScaled`]: crate::backend::ScoreScaled
//! [`Recognizer<Text>`]: elide_core::recognition::Recognizer

use elide_core::backend::{Backend, BackendBatchExt};
use elide_core::entity::audit::{AuditEvent, ModelId};
use elide_core::entity::{Entity, Label};
use elide_core::modality::TextRecognizable;
use elide_core::recognition::{Context, Recognizer, Subject};
use elide_core::{Component, Result};
use hipstr::HipStr;

#[cfg(any(test, feature = "mocks"))]
use crate::backend::MockBackend;
use crate::backend::{NerRequest, NerResponse, NerSpan};

const TARGET: &str = "elide_ner::recognition";

/// A recognizer that runs a NER model over text, through a NER backend.
///
/// Generic over its backend `B`: any [`Backend`] whose request is
/// [`NerRequest`] and whose response is [`NerResponse`]. Each entity's audit
/// event names the model its response came from, so entities stay attributed
/// to the model that answered behind a router.
#[derive(Clone, Component)]
pub struct NerRecognizer<B> {
    /// Recognizer name, its id; the crate name unless set.
    #[component(name)]
    name: HipStr<'static>,
    /// Backend that turns `(text, labels)` into canonical spans. May be a
    /// decorated backend.
    backend: B,
}

impl<B> NerRecognizer<B>
where
    B: for<'a> Backend<Request<'a> = NerRequest<'a>, Response = NerResponse>,
{
    /// A NER recognizer over `backend`, named after the crate until
    /// [`with_name`](Self::with_name) sets another.
    #[must_use]
    pub fn new(backend: B) -> Self {
        Self {
            name: HipStr::borrowed(env!("CARGO_PKG_NAME")),
            backend,
        }
    }

    /// Set the recognizer name, surfaced as its id; set one to tell several NER
    /// recognizers apart.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<HipStr<'static>>) -> Self {
        self.name = name.into();
        self
    }
}

#[cfg(any(test, feature = "mocks"))]
impl NerRecognizer<MockBackend> {
    /// A NER recognizer over the no-op [`MockBackend`], which produces no spans.
    ///
    /// [`MockBackend`]: crate::backend::MockBackend
    #[cfg_attr(docsrs, doc(cfg(feature = "mocks")))]
    #[must_use]
    pub fn mock() -> Self {
        Self::new(MockBackend::new())
    }
}

impl<B> NerRecognizer<B> {
    /// Recognizer name: the one set with [`with_name`](Self::with_name), or the
    /// crate name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[async_trait::async_trait]
impl<M, B> Recognizer<M> for NerRecognizer<B>
where
    M: TextRecognizable,
    B: for<'a> Backend<Request<'a> = NerRequest<'a>, Response = NerResponse>,
{
    async fn recognize(&self, subject: &Subject<M>, ctx: &Context<'_>) -> Result<Vec<Entity<M>>> {
        // No recognizable text at this chunk (an un-transcribed clip, an
        // un-OCR'd image): skip the model call and recognize nothing.
        let labels = ctx.target_label_defs();
        let Some(request) = ner_request(subject, ctx, &labels) else {
            return Ok(Vec::new());
        };
        let response = self.backend.call(request).await?;
        let spans = response.spans.len();
        let entities = entities(response.spans, subject, &response.model);
        tracing::debug!(
            target: TARGET,
            recognizer = %self.name,
            model = %response.model.name,
            spans,
            entities = entities.len(),
            unplaced = spans - entities.len(),
            "ner spans placed",
        );
        Ok(entities)
    }

    async fn recognize_batch(
        &self,
        subjects: &[&Subject<M>],
        ctx: &Context<'_>,
    ) -> Result<Vec<Vec<Entity<M>>>> {
        // One NER request per subject with recognizable text; no-text subjects are
        // skipped and recognize nothing. `targets` keeps each request aligned with
        // its subject index, so a response scatters back to the right subject.
        let labels = ctx.target_label_defs();
        let mut requests = Vec::new();
        let mut targets = Vec::new();
        for (index, subject) in subjects.iter().enumerate() {
            if let Some(request) = ner_request(subject, ctx, &labels) {
                requests.push(request);
                targets.push(index);
            }
        }

        let mut per_subject: Vec<Vec<Entity<M>>> = subjects.iter().map(|_| Vec::new()).collect();
        if requests.is_empty() {
            return Ok(per_subject);
        }
        let responses = self.backend.call_aligned(requests).await?;
        let spans: usize = responses.iter().map(|response| response.spans.len()).sum();
        for (&index, response) in targets.iter().zip(responses) {
            per_subject[index] = entities(response.spans, subjects[index], &response.model);
        }
        let placed: usize = per_subject.iter().map(Vec::len).sum();
        tracing::debug!(
            target: TARGET,
            recognizer = %self.name,
            subjects = subjects.len(),
            requests = targets.len(),
            spans,
            entities = placed,
            unplaced = spans - placed,
            "ner spans placed",
        );
        Ok(per_subject)
    }
}

/// The entities for `spans`, located in `subject` and attributed to
/// `model`. A span whose range can't be placed in the medium (an
/// OCR/transcript range no enrichment covers) is dropped.
fn entities<M: TextRecognizable>(
    spans: Vec<NerSpan>,
    subject: &Subject<M>,
    model: &ModelId,
) -> Vec<Entity<M>> {
    spans
        .into_iter()
        .filter_map(|span| {
            let location = M::locate(span.offset.clone(), subject.data(), subject.artifact())?;
            let event = AuditEvent::model("ner", span.confidence, location.clone(), model.clone());
            Some(
                Entity::new(span.label, location)
                    .with_recognized_range(span.offset)
                    .with_event(event),
            )
        })
        .collect()
}

/// The per-call NER request for `subject`, or `None` when it carries no
/// recognizable text (an un-transcribed clip, an un-OCR'd image).
fn ner_request<'a, M: TextRecognizable>(
    subject: &'a Subject<M>,
    ctx: &'a Context<'_>,
    labels: &'a [Label],
) -> Option<NerRequest<'a>> {
    let text = M::as_text(subject.data(), subject.artifact())?;
    Some(NerRequest {
        text,
        labels: (!labels.is_empty()).then_some(labels),
        language: ctx.languages(subject).primary(),
        correlation_id: ctx.correlation_id(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use elide_core::ErrorKind;
    use elide_core::entity::LabelCatalog;
    use elide_core::entity::audit::AuditKind;
    use elide_core::modality::text::{Text, TextData};
    use elide_core::recognition::Scope;

    use super::*;

    #[tokio::test]
    async fn mock_backend_yields_no_entities() {
        let rec = NerRecognizer::mock().with_name("test");
        let data = TextData::new("Alice Smith".to_owned());
        let scope = Scope::new().with_catalog(LabelCatalog::with_builtins());
        let ctx = Context::new(&scope);
        let subject = Subject::<Text>::new(data);
        let out = rec.recognize(&subject, &ctx).await.unwrap();
        assert!(out.is_empty());
    }

    /// A backend recording how it was called, to prove `recognize_batch`
    /// coalesced N subjects into one `call_batch` rather than N single calls.
    /// Counters are `Arc`-shared so the test can read them after the backend is
    /// moved into the recognizer.
    #[derive(Clone, Default)]
    struct BatchCounting {
        batch_calls: Arc<AtomicUsize>,
        single_calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Backend for BatchCounting {
        type Request<'a> = NerRequest<'a>;
        type Response = NerResponse;

        async fn call(&self, _request: NerRequest<'_>) -> Result<NerResponse> {
            self.single_calls.fetch_add(1, Ordering::Relaxed);
            Ok(NerResponse::new(
                ModelId::new("batch-counting"),
                vec![NerSpan::new("EMAIL", 0.9, 0..1)],
            ))
        }

        async fn call_batch(&self, requests: Vec<NerRequest<'_>>) -> Result<Vec<NerResponse>> {
            self.batch_calls.fetch_add(1, Ordering::Relaxed);
            Ok(requests
                .iter()
                .map(|_| {
                    NerResponse::new(
                        ModelId::new("batch-counting"),
                        vec![NerSpan::new("EMAIL", 0.9, 0..1)],
                    )
                })
                .collect())
        }
    }

    #[tokio::test]
    async fn recognize_batch_coalesces_across_subjects() {
        let backend = BatchCounting::default();
        let counters = backend.clone();
        let recognizer = NerRecognizer::new(backend);
        let scope = Scope::new();
        let ctx = Context::new(&scope);
        let subjects: [Subject<Text>; _] = [
            Subject::new(TextData::new("a".to_owned())),
            Subject::new(TextData::new("b".to_owned())),
            Subject::new(TextData::new("c".to_owned())),
        ];

        let out = recognizer
            .recognize_batch(&subjects.iter().collect::<Vec<_>>(), &ctx)
            .await
            .unwrap();

        // One batched round-trip covered all three subjects; no single calls.
        assert_eq!(counters.batch_calls.load(Ordering::Relaxed), 1);
        assert_eq!(counters.single_calls.load(Ordering::Relaxed), 0);
        // One recognition per subject, each with the backend's one span.
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|entities| entities.len() == 1));
    }

    /// A backend that shorts the batch must error, not silently leave the tail
    /// subjects unrecognized: those subjects would keep an empty entity list and
    /// their PII would go undetected with no error.
    #[tokio::test]
    async fn recognize_batch_rejects_a_short_response() {
        let recognizer = NerRecognizer::new(MockBackend::new().with_dropped_responses(1));
        let scope = Scope::new();
        let ctx = Context::new(&scope);
        let subjects: [Subject<Text>; _] = [
            Subject::new(TextData::new("a".to_owned())),
            Subject::new(TextData::new("b".to_owned())),
        ];

        let err = recognizer
            .recognize_batch(&subjects.iter().collect::<Vec<_>>(), &ctx)
            .await
            .expect_err("a short batch response is a contract violation");
        assert_eq!(err.kind(), ErrorKind::Provider);
    }

    /// Every span becomes an entity, in the catalog or not, and its audit event
    /// names the model the backend reports.
    #[tokio::test]
    async fn entities_name_the_backend_model_and_keep_out_of_catalog_spans() {
        let recognizer = NerRecognizer::new(BatchCounting::default()).with_name("named");
        let scope = Scope::new().with_catalog(LabelCatalog::with_builtins());
        let ctx = Context::new(&scope);
        let subject = Subject::<Text>::new(TextData::new("a".to_owned()));

        let out = recognizer.recognize(&subject, &ctx).await.unwrap();

        assert_eq!(out.len(), 1, "the out-of-catalog EMAIL span is kept");
        let AuditKind::Model(event) = &out[0].audit.events()[0].kind else {
            panic!("a model event");
        };
        assert_eq!(event.model.name, "batch-counting");
    }
}
