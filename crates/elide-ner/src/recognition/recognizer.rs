//! [`NerRecognizer`]: unified NER recognizer that drives any
//! NER backend.
//!
//! Holds its NER backend `B` plus the recognizer's advertised
//! [`supported_labels`]. On each `recognize` call it asks the backend for
//! spans (passing `Some(&labels)` when non-empty for zero-shot backends,
//! `None` when empty for fixed-label backends), then emits entities from
//! the canonical spans the backend returns. Filtering ignored labels or
//! scaling scores is the job of backend decorators
//! ([`IgnoreLabels`], [`ScoreScale`]), not the recognizer.
//!
//! Implements [`Recognizer<Text>`] so it composes with the
//! rest of the platform through the same trait every other text
//! recognizer uses.
//!
//! [`supported_labels`]: NerRecognizer::supported_labels
//! [`IgnoreLabels`]: crate::decorator::IgnoreLabels
//! [`ScoreScale`]: crate::decorator::ScoreScale
//! [`Recognizer<Text>`]: elide_core::recognition::Recognizer

use elide_core::backend::Backend;
use elide_core::entity::audit::{AuditEvent, ModelEvent};
use elide_core::entity::{Entity, Label, LabelCatalog, LabelRef};
use elide_core::modality::TextRecognizable;
use elide_core::primitive::ComponentId;
use elide_core::recognition::{Context, Recognizer, Subject};
use elide_core::{Error, ErrorKind, Result};
use hipstr::HipStr;

use super::aggregation::AggregationStrategy;
use super::alignment::AlignmentMode;
#[cfg(any(test, feature = "mocks"))]
use crate::backend::MockBackend;
use crate::backend::{NerRequest, NerResponse, NerSpan};

/// Trait-driven NER recognizer, generic over its NER backend `B` — any
/// [`Backend`](elide_core::backend::Backend) whose request is [`NerRequest`] and
/// whose response is [`NerResponse`].
#[derive(Clone)]
pub struct NerRecognizer<B = ()> {
    /// Optional recognizer name, surfaced in the recognition event on every
    /// emitted entity. `None` falls back to the crate name at [`id`](Recognizer::id)
    /// time; set one to tell several NER recognizers apart.
    name: Option<HipStr<'static>>,
    /// Backend that turns `(text, kinds)` into raw spans. May be a
    /// `Metered` wrapper.
    backend: B,
    /// Labels the recognizer advertises. When non-empty, the
    /// recognizer asks the backend for only this subset on every
    /// call (zero-shot path). When empty, the backend is asked for
    /// whatever it natively produces (fixed-label path).
    supported_labels: Vec<LabelRef>,
    /// Aggregation policy for backends that emit token-level
    /// predictions. Advisory for backends that aggregate server-side.
    aggregation: AggregationStrategy,
    /// Alignment policy for sub-word predictions. Same advisory
    /// status as `aggregation`.
    alignment: AlignmentMode,
}

impl<B> NerRecognizer<B>
where
    B: for<'a> Backend<Request<'a> = NerRequest<'a>, Response = NerResponse>,
{
    /// A NER recognizer over `backend`, with no advertised labels (fixed-label
    /// path) and default aggregation/alignment.
    ///
    /// Unnamed by default (its id falls back to the crate name); refine with the
    /// `with_*` setters.
    #[must_use]
    pub fn new(backend: B) -> Self {
        Self {
            name: None,
            backend,
            supported_labels: Vec::new(),
            aggregation: AggregationStrategy::default(),
            alignment: AlignmentMode::default(),
        }
    }

    /// Set the recognizer name, surfaced as its id.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<HipStr<'static>>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Advertise the labels the recognizer requests (the zero-shot subset).
    #[must_use]
    pub fn with_supported_labels(mut self, labels: impl Into<Vec<LabelRef>>) -> Self {
        self.supported_labels = labels.into();
        self
    }

    /// Set the aggregation policy for token-level backends.
    #[must_use]
    pub fn with_aggregation(mut self, aggregation: AggregationStrategy) -> Self {
        self.aggregation = aggregation;
        self
    }

    /// Set the alignment policy for sub-word backends.
    #[must_use]
    pub fn with_alignment(mut self, alignment: AlignmentMode) -> Self {
        self.alignment = alignment;
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
    /// Recognizer name — the one set with [`with_name`](Self::with_name), or the
    /// crate name when unset.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_deref().unwrap_or(env!("CARGO_PKG_NAME"))
    }

    /// Labels this recognizer advertises.
    #[must_use]
    pub fn supported_labels(&self) -> &[LabelRef] {
        &self.supported_labels
    }

    /// The target labels for a call, resolved against `catalog`, as full
    /// [`Label`]s (localized name + optional description) so a zero-shot backend
    /// gets the localized text in the analysis language.
    ///
    /// With no configured [`supported_labels`](Self::supported_labels) the whole
    /// catalog is the target. Otherwise the recognizer's own set overrides,
    /// each entry resolved against the catalog; a supported label *absent* from
    /// the catalog is dropped, there is no localized definition to send, and
    /// fabricating one from its id would mislead the backend. An empty result
    /// leaves the backend to emit whatever it natively produces.
    fn effective_labels(&self, catalog: &LabelCatalog) -> Vec<Label> {
        if self.supported_labels.is_empty() {
            catalog.iter().cloned().collect()
        } else {
            self.supported_labels
                .iter()
                .filter_map(|r| catalog.get(r).cloned())
                .collect()
        }
    }

    /// Aggregation policy for token-level backends.
    #[must_use]
    pub fn aggregation(&self) -> AggregationStrategy {
        self.aggregation
    }

    /// Alignment policy for sub-word backends.
    #[must_use]
    pub fn alignment(&self) -> AlignmentMode {
        self.alignment
    }

    /// Place a backend [`NerSpan`] into a located [`Entity`] carrying a
    /// [`Model`] birth event, keeping the span's byte offset as the entity's
    /// `recognized_range`. Drops the match (`None`) when its range can't be
    /// placed in the medium (an OCR/transcript range no enrichment covers).
    ///
    /// [`Model`]: elide_core::entity::audit::AuditKind::Model
    fn build_entity<M: TextRecognizable>(
        &self,
        span: &NerSpan,
        label: LabelRef,
        subject: &Subject<M>,
    ) -> Option<Entity<M>> {
        let range = span.offset.clone();
        let location = M::locate(range.clone(), subject.data(), subject.artifact())?;
        let event = AuditEvent::model(
            "ner",
            span.confidence,
            location.clone(),
            ModelEvent {
                name: HipStr::from(self.name().to_owned()),
                ..ModelEvent::default()
            },
        );
        Some(
            Entity::builder()
                .with_label(label)
                .with_location(location)
                .with_confidence(span.confidence)
                .with_recognized_range(range)
                .with_event(event)
                .build()
                .expect("required fields provided"),
        )
    }
}

#[async_trait::async_trait]
impl<M, B> Recognizer<M> for NerRecognizer<B>
where
    M: TextRecognizable,
    B: for<'a> Backend<Request<'a> = NerRequest<'a>, Response = NerResponse>,
{
    fn id(&self) -> ComponentId {
        ComponentId::new(self.name().to_owned(), env!("CARGO_PKG_VERSION"))
    }

    async fn recognize(
        &self,
        subject: &Subject<M>,
        ctx: &Context<'_, M>,
    ) -> Result<Vec<Entity<M>>> {
        // No recognizable text at this chunk (an un-transcribed clip, an
        // un-OCR'd image): skip the model call and recognize nothing.
        let effective_labels = self.effective_labels(ctx.catalog());
        let Some(request) = ner_request(subject, ctx, &effective_labels) else {
            return Ok(Vec::new());
        };
        let response = self.backend.call(request).await?;
        Ok(self.spans_to_entities(&response.spans, &effective_labels, subject))
    }

    async fn recognize_batch(
        &self,
        subjects: &[Subject<M>],
        ctx: &Context<'_, M>,
    ) -> Result<Vec<Vec<Entity<M>>>> {
        // One NER request per subject with recognizable text; no-text subjects are
        // skipped and recognize nothing. `targets` keeps each request aligned with
        // its subject index, so a response scatters back to the right subject.
        let effective_labels = self.effective_labels(ctx.catalog());
        let mut requests = Vec::new();
        let mut targets = Vec::new();
        for (index, subject) in subjects.iter().enumerate() {
            if let Some(request) = ner_request(subject, ctx, &effective_labels) {
                requests.push(request);
                targets.push(index);
            }
        }

        let mut per_subject: Vec<Vec<Entity<M>>> = subjects.iter().map(|_| Vec::new()).collect();
        if requests.is_empty() {
            return Ok(per_subject);
        }
        // One entity list per subject is this method's contract; the backend
        // must answer one response per request to honor it. A short batch would
        // otherwise let the zip silently leave the tail subjects unrecognized.
        let responses = self.backend.call_batch(requests).await?;
        if responses.len() != targets.len() {
            return Err(Error::new(
                ErrorKind::Provider,
                format!(
                    "NER backend returned {} responses for {} requests",
                    responses.len(),
                    targets.len()
                ),
            ));
        }
        for (&index, response) in targets.iter().zip(responses) {
            per_subject[index] =
                self.spans_to_entities(&response.spans, &effective_labels, &subjects[index]);
        }
        Ok(per_subject)
    }
}

impl<B> NerRecognizer<B> {
    /// Map a response's `spans` to entities located in `subject`.
    ///
    /// Spans already carry canonical labels (the backend did any raw-to-canonical
    /// mapping; ignored labels are dropped by an `IgnoreLabels` decorator). When a
    /// target set was requested, only those labels survive. Each surviving span is
    /// placed in the medium; one whose range can't be located is dropped.
    fn spans_to_entities<M: TextRecognizable>(
        &self,
        spans: &[NerSpan],
        effective_labels: &[Label],
        subject: &Subject<M>,
    ) -> Vec<Entity<M>> {
        spans
            .iter()
            .filter(|s| {
                effective_labels.is_empty()
                    || effective_labels.iter().any(|l| l.to_ref() == s.label)
            })
            .filter_map(|s| self.build_entity::<M>(s, s.label.clone(), subject))
            .collect()
    }
}

/// The per-call NER request for `subject`, or `None` when it carries no
/// recognizable text (an un-transcribed clip, an un-OCR'd image).
fn ner_request<'a, M: TextRecognizable>(
    subject: &'a Subject<M>,
    ctx: &'a Context<'_, M>,
    effective_labels: &'a [Label],
) -> Option<NerRequest<'a>> {
    let text = M::as_text(subject.data(), subject.artifact())?;
    let labels = (!effective_labels.is_empty()).then_some(effective_labels);
    Some(NerRequest {
        text,
        labels,
        language: ctx.languages(subject).primary(),
        correlation_id: ctx.correlation_id(),
    })
}

#[cfg(test)]
mod tests {
    use elide_core::entity::{LabelCatalog, LabelLocale, builtins};
    use elide_core::modality::text::{Text, TextData};
    use elide_core::primitive::LanguageTag;
    use elide_core::recognition::Scope;

    use super::*;

    #[tokio::test]
    async fn mock_backend_yields_no_entities() {
        let rec = NerRecognizer::mock()
            .with_name("test")
            .with_supported_labels(vec![
                builtins::PERSON_NAME.to_ref(),
                builtins::EMAIL_ADDRESS.to_ref(),
            ]);
        let data = TextData::new("Alice Smith".to_owned());
        let scope = Scope::new();
        let ctx = Context::<Text>::new(&scope);
        let subject = Subject::new(data);
        let out = rec.recognize(&subject, &ctx).await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn empty_supported_labels_passes_none_to_backend() {
        let rec = NerRecognizer::mock().with_name("test");
        let data = TextData::new("Alice Smith".to_owned());
        let scope = Scope::new();
        let ctx = Context::<Text>::new(&scope);
        let subject = Subject::new(data);
        let out = rec.recognize(&subject, &ctx).await.unwrap();
        assert!(out.is_empty());
    }

    /// A recognizer with the given `supported_labels`, for testing
    /// [`effective_labels`](NerRecognizer::effective_labels) directly.
    fn recognizer_with(supported: Vec<LabelRef>) -> NerRecognizer<MockBackend> {
        NerRecognizer::mock()
            .with_name("test")
            .with_supported_labels(supported)
    }

    #[test]
    fn no_supported_labels_targets_the_whole_catalog() {
        // With no configured set, every catalog label is a target, as a full
        // `Label`, so a zero-shot backend gets the localized name *and* the
        // description.
        let mut catalog = LabelCatalog::new();
        catalog.insert(Label::new("email", "email address").with_localization(
            LanguageTag::english(),
            LabelLocale::described("email address", "an email address"),
        ));
        let rec = recognizer_with(vec![]);

        let en = LanguageTag::english();
        let labels = rec.effective_labels(&catalog);
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].name(&en), "email address");
        assert!(labels[0].description(&en).is_some());
    }

    #[test]
    fn supported_labels_select_a_subset_of_the_catalog() {
        // The catalog carries both; the recognizer's own set restricts to just
        // person_name, resolved to its catalog definition.
        let mut catalog = LabelCatalog::new();
        catalog.insert(Label::new("email", "email address"));
        catalog.insert((*builtins::PERSON_NAME).clone());
        let rec = recognizer_with(vec![builtins::PERSON_NAME.to_ref()]);

        let en = LanguageTag::english();
        let labels = rec.effective_labels(&catalog);
        let names: Vec<&str> = labels.iter().map(|l| l.name(&en)).collect();
        assert_eq!(names, vec![builtins::PERSON_NAME.name(&en)]);
    }

    #[test]
    fn supported_label_absent_from_catalog_is_dropped() {
        // person_name is not in the catalog, so there is no localized
        // definition to send, it is dropped, not fabricated from its id.
        let mut catalog = LabelCatalog::new();
        catalog.insert(Label::new("email", "email address"));
        let rec = recognizer_with(vec![builtins::PERSON_NAME.to_ref()]);

        assert!(rec.effective_labels(&catalog).is_empty());
    }

    /// A backend recording how it was called, to prove `recognize_batch`
    /// coalesced N subjects into one `call_batch` rather than N single calls.
    /// Counters are `Arc`-shared so the test can read them after the backend is
    /// moved into the recognizer.
    #[derive(Clone, Default)]
    struct BatchCounting {
        batch_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        single_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Backend for BatchCounting {
        type Request<'a> = NerRequest<'a>;
        type Response = NerResponse;

        fn provenance(&self) -> ModelEvent {
            ModelEvent {
                name: "batch-counting".into(),
                ..ModelEvent::default()
            }
        }

        async fn call(&self, _request: NerRequest<'_>) -> Result<NerResponse> {
            self.single_calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(NerResponse::new(vec![NerSpan::new("EMAIL", 0.9, 0..1)]))
        }

        async fn call_batch(&self, requests: Vec<NerRequest<'_>>) -> Result<Vec<NerResponse>> {
            self.batch_calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(requests
                .iter()
                .map(|_| NerResponse::new(vec![NerSpan::new("EMAIL", 0.9, 0..1)]))
                .collect())
        }
    }

    #[tokio::test]
    async fn recognize_batch_coalesces_across_subjects() {
        use std::sync::atomic::Ordering;

        let backend = BatchCounting::default();
        let counters = backend.clone();
        let recognizer = NerRecognizer::new(backend);
        let scope = Scope::new();
        let ctx = Context::<Text>::new(&scope);
        let subjects = [
            Subject::new(TextData::new("a".to_owned())),
            Subject::new(TextData::new("b".to_owned())),
            Subject::new(TextData::new("c".to_owned())),
        ];

        let out = recognizer.recognize_batch(&subjects, &ctx).await.unwrap();

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
        let ctx = Context::<Text>::new(&scope);
        let subjects = [
            Subject::new(TextData::new("a".to_owned())),
            Subject::new(TextData::new("b".to_owned())),
        ];

        let err = recognizer
            .recognize_batch(&subjects, &ctx)
            .await
            .expect_err("a short batch response is a contract violation");
        assert_eq!(err.kind(), ErrorKind::Provider);
    }
}
