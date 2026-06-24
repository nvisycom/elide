//! [`NerRecognizer`]: unified NER recognizer that drives any
//! [`NerBackend`] backend.
//!
//! Holds an `Arc<dyn NerBackend>` plus the recognizer's advertised
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

use std::sync::Arc;

use derive_builder::Builder;
use elide_core::entity::provenance::{Event, ModelEvent};
use elide_core::entity::{Entity, Label, LabelRef};
use elide_core::modality::TextRecognizable;
use elide_core::recognition::{Recognizer, RecognizerContext, RecognizerId};
use elide_core::{Error, Result};
use hipstr::HipStr;

use super::aggregation::AggregationStrategy;
use super::alignment::AlignmentMode;
#[cfg(any(test, feature = "mock"))]
use crate::backend::MockBackend;
use crate::backend::{NerBackend, NerRequest, NerSpan};

/// Trait-driven NER recognizer.
#[derive(Clone, Builder)]
#[builder(
    name = "NerRecognizerBuilder",
    pattern = "owned",
    setter(into, prefix = "with"),
    build_fn(error = "Error", name = "try_build", private)
)]
pub struct NerRecognizer {
    /// Recognizer name. Surfaced in the recognition event on every
    /// emitted entity, so cheap to clone and never changed after
    /// construction.
    name: HipStr<'static>,
    /// Backend that turns `(text, kinds)` into raw spans. Required.
    /// Set via [`with_backend`], which accepts any concrete
    /// [`NerBackend`] impl by value and wraps it in `Arc` internally.
    ///
    /// [`with_backend`]: NerRecognizerBuilder::with_backend
    #[builder(setter(custom))]
    backend: Arc<dyn NerBackend>,
    /// Labels the recognizer advertises. When non-empty, the
    /// recognizer asks the backend for only this subset on every
    /// call (zero-shot path). When empty, the backend is asked for
    /// whatever it natively produces (fixed-label path).
    #[builder(default)]
    supported_labels: Vec<LabelRef>,
    /// Aggregation policy for backends that emit token-level
    /// predictions. Advisory for backends that aggregate server-side.
    #[builder(default)]
    aggregation: AggregationStrategy,
    /// Alignment policy for sub-word predictions. Same advisory
    /// status as `aggregation`.
    #[builder(default)]
    alignment: AlignmentMode,
}

impl NerRecognizer {
    /// Start the chainable builder. `name` and `backend` are
    /// required; calling [`build`] without them returns a
    /// validation error.
    ///
    /// [`build`]: NerRecognizerBuilder::build
    #[must_use]
    pub fn builder() -> NerRecognizerBuilder {
        NerRecognizerBuilder::default()
    }

    /// Recognizer name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Labels this recognizer advertises.
    #[must_use]
    pub fn supported_labels(&self) -> &[LabelRef] {
        &self.supported_labels
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
    /// [`Model`]: elide_core::entity::provenance::EventKind::Model
    fn build_entity<M: TextRecognizable>(
        &self,
        span: &NerSpan,
        label: LabelRef,
        data: &M::Data,
        ctx: &RecognizerContext<'_, M>,
    ) -> Option<Entity<M>> {
        let range = span.offset.clone();
        let location = M::locate(range.clone(), data, &ctx.artifacts)?;
        let reason = format!("recognizer `{}` identified {}", self.name, label.as_str());
        let event = Event::model(
            "ner",
            span.confidence,
            location.clone(),
            ModelEvent {
                name: self.name.clone(),
                ..ModelEvent::default()
            },
        )
        .with_reason(reason);
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

impl NerRecognizerBuilder {
    /// Set the [`NerBackend`] that powers this recognizer. Accepts any
    /// concrete impl by value and wraps it in `Arc`. Required: `build`
    /// errors when this hasn't been called.
    #[must_use]
    pub fn with_backend<B: NerBackend>(mut self, backend: B) -> Self {
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
    pub fn with_mock_backend(self) -> Self {
        self.with_backend(MockBackend)
    }

    /// Finish the builder. Errors when `name` or `backend` is unset.
    pub fn build(self) -> Result<NerRecognizer> {
        self.try_build()
    }
}

impl<M: TextRecognizable> Recognizer<M> for NerRecognizer {
    fn id(&self) -> RecognizerId {
        RecognizerId::new(self.name.clone(), env!("CARGO_PKG_VERSION"))
    }

    async fn recognize(
        &self,
        data: &M::Data,
        ctx: &RecognizerContext<'_, M>,
    ) -> Result<Vec<Entity<M>>> {
        // The effective target labels, as full `Label`s (name +
        // description) so a zero-shot backend like GLiNER 2.0 gets the
        // descriptions: the recognizer's own configured set overrides when
        // present, else the run-wide catalog from the scope. Descriptions
        // for the override path are resolved against the catalog; a label
        // absent from it falls back to a description-less `Label`. Empty
        // leaves the backend to emit whatever it natively produces.
        let effective_labels: Vec<Label> = if self.supported_labels.is_empty() {
            ctx.catalog().iter().cloned().collect()
        } else {
            self.supported_labels
                .iter()
                .map(|r| {
                    ctx.catalog()
                        .get(r)
                        .cloned()
                        .unwrap_or_else(|| Label::new(r.as_str()))
                })
                .collect()
        };
        let labels = if effective_labels.is_empty() {
            None
        } else {
            Some(effective_labels.as_slice())
        };
        let request = NerRequest {
            text: M::as_text(data, &ctx.artifacts),
            labels,
            language: ctx.primary_language(),
            correlation_id: ctx.correlation_id(),
        };
        let response = self.backend.recognize(request).await?;

        // Spans already carry canonical labels (the backend did any
        // raw-to-canonical mapping; ignored labels are dropped by an
        // `IgnoreLabels` decorator). When a target set was requested, we
        // restrict to it. Each surviving span is placed in the medium; one
        // whose range can't be located is dropped.
        Ok(response
            .spans
            .iter()
            .filter(|s| {
                effective_labels.is_empty()
                    || effective_labels.iter().any(|l| l.to_ref() == s.label)
            })
            .filter_map(|s| self.build_entity::<M>(s, s.label.clone(), data, ctx))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use elide_core::entity::{LabelCatalog, builtins};
    use elide_core::modality::text::{Text, TextData};
    use elide_core::recognition::Scope;

    use super::*;
    use crate::backend::NerResponse;

    #[tokio::test]
    async fn mock_backend_yields_no_entities() {
        let rec = NerRecognizer::builder()
            .with_name("test")
            .with_mock_backend()
            .with_supported_labels(vec![
                builtins::PERSON_NAME.to_ref(),
                builtins::EMAIL_ADDRESS.to_ref(),
            ])
            .build()
            .expect("builder succeeds");
        let data = TextData::new("Alice Smith".to_owned());
        let scope = Scope::<Text>::new();
        let ctx = RecognizerContext::new(&scope);
        let out = rec.recognize(&data, &ctx).await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn empty_supported_labels_passes_none_to_backend() {
        let rec = NerRecognizer::builder()
            .with_name("test")
            .with_mock_backend()
            .build()
            .expect("builder succeeds");
        let data = TextData::new("Alice Smith".to_owned());
        let scope = Scope::<Text>::new();
        let ctx = RecognizerContext::new(&scope);
        let out = rec.recognize(&data, &ctx).await.unwrap();
        assert!(out.is_empty());
    }

    /// Each captured label: its name and whether it carried a description.
    type SeenLabels = Arc<Mutex<Option<Vec<(String, bool)>>>>;

    /// Backend that records the labels it received, so a test can assert
    /// what the recognizer sent.
    #[derive(Clone, Default)]
    struct CapturingBackend {
        seen: SeenLabels,
    }

    #[async_trait::async_trait]
    impl NerBackend for CapturingBackend {
        fn provenance(&self) -> ModelEvent {
            ModelEvent {
                name: "capturing".into(),
                ..ModelEvent::default()
            }
        }

        async fn recognize(&self, request: NerRequest<'_>) -> Result<NerResponse> {
            *self.seen.lock().unwrap() = request.labels.map(|labels| {
                labels
                    .iter()
                    .map(|l| (l.name().to_owned(), l.description().is_some()))
                    .collect()
            });
            Ok(NerResponse::default())
        }
    }

    #[tokio::test]
    async fn catalog_on_scope_becomes_the_target_labels() {
        let backend = CapturingBackend::default();
        let seen = backend.seen.clone();
        let rec = NerRecognizer::builder()
            .with_name("test")
            .with_backend(backend)
            .build()
            .expect("builder succeeds");

        // No `with_supported_labels`: the scope's catalog drives the labels,
        // carrying descriptions for a zero-shot backend.
        let mut catalog = LabelCatalog::new();
        catalog.insert(Label::described("EMAIL", "an email address"));
        let scope = Scope::<Text>::new().with_catalog(catalog);
        let ctx = RecognizerContext::new(&scope);
        rec.recognize(&TextData::new("x".to_owned()), &ctx)
            .await
            .unwrap();

        let seen = seen.lock().unwrap().clone().expect("labels were sent");
        assert_eq!(seen, vec![("EMAIL".to_owned(), true)]);
    }

    #[tokio::test]
    async fn supported_labels_override_the_catalog() {
        let backend = CapturingBackend::default();
        let seen = backend.seen.clone();
        let rec = NerRecognizer::builder()
            .with_name("test")
            .with_backend(backend)
            .with_supported_labels(vec![builtins::PERSON_NAME.to_ref()])
            .build()
            .expect("builder succeeds");

        // The catalog is present but the recognizer's own set overrides it.
        let mut catalog = LabelCatalog::new();
        catalog.insert(Label::described("EMAIL", "an email address"));
        let scope = Scope::<Text>::new().with_catalog(catalog);
        let ctx = RecognizerContext::new(&scope);
        rec.recognize(&TextData::new("x".to_owned()), &ctx)
            .await
            .unwrap();

        let seen = seen.lock().unwrap().clone().expect("labels were sent");
        let names: Vec<&str> = seen.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec![builtins::PERSON_NAME.name()]);
    }
}
