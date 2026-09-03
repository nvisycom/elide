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
use elide_core::entity::audit::{AuditEvent, ModelEvent};
use elide_core::entity::{Entity, Label, LabelCatalog, LabelRef};
use elide_core::modality::TextRecognizable;
#[cfg(feature = "usage")]
use elide_core::primitive::ModelUsage;
use elide_core::recognition::{Recognition, Recognizer, RecognizerContext, RecognizerId};
use elide_core::{Error, Result};
use hipstr::HipStr;

use super::aggregation::AggregationStrategy;
use super::alignment::AlignmentMode;
#[cfg(any(test, feature = "test-utils"))]
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
        data: &M::Data,
        ctx: &RecognizerContext<'_, M>,
    ) -> Option<Entity<M>> {
        let range = span.offset.clone();
        let location = M::locate(range.clone(), data, ctx.artifact())?;
        let event = AuditEvent::model(
            "ner",
            span.confidence,
            location.clone(),
            ModelEvent {
                name: self.name.clone(),
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
    #[cfg(any(test, feature = "test-utils"))]
    #[cfg_attr(docsrs, doc(cfg(feature = "test-utils")))]
    #[must_use]
    pub fn with_mock_backend(self) -> Self {
        self.with_backend(MockBackend)
    }

    /// Finish the builder. Errors when `name` or `backend` is unset.
    pub fn build(self) -> Result<NerRecognizer> {
        self.try_build()
    }
}

#[async_trait::async_trait]
impl<M: TextRecognizable> Recognizer<M> for NerRecognizer {
    fn id(&self) -> RecognizerId {
        RecognizerId::new(self.name.clone(), env!("CARGO_PKG_VERSION"))
    }

    async fn recognize(
        &self,
        data: &M::Data,
        ctx: &RecognizerContext<'_, M>,
    ) -> Result<Recognition<M>> {
        let effective_labels = self.effective_labels(ctx.catalog());
        let labels = if effective_labels.is_empty() {
            None
        } else {
            Some(effective_labels.as_slice())
        };
        let request = NerRequest {
            text: M::as_text(data, ctx.artifact()),
            labels,
            language: ctx.primary_language(),
            correlation_id: ctx.correlation_id(),
        };
        let response = self.backend.recognize(request).await?;

        // The model that produced these spans vouches for its own identity
        // (name + version) via `provenance()`, plus any tokens it reported.
        #[cfg(feature = "usage")]
        let model_usage = ModelUsage::from(self.backend.provenance()).with_tokens(response.tokens);

        // Spans already carry canonical labels (the backend did any
        // raw-to-canonical mapping; ignored labels are dropped by an
        // `IgnoreLabels` decorator). When a target set was requested, we
        // restrict to it. Each surviving span is placed in the medium; one
        // whose range can't be located is dropped.
        let entities = response
            .spans
            .iter()
            .filter(|s| {
                effective_labels.is_empty()
                    || effective_labels.iter().any(|l| l.to_ref() == s.label)
            })
            .filter_map(|s| self.build_entity::<M>(s, s.label.clone(), data, ctx))
            .collect();
        let recognition = Recognition::new(entities);
        #[cfg(feature = "usage")]
        let recognition = recognition.with_model_usage(model_usage);
        Ok(recognition)
    }
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
        let scope = Scope::new();
        let ctx = RecognizerContext::<Text>::new(&scope);
        let out = rec.recognize(&data, &ctx).await.unwrap().entities;
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
        let scope = Scope::new();
        let ctx = RecognizerContext::<Text>::new(&scope);
        let out = rec.recognize(&data, &ctx).await.unwrap().entities;
        assert!(out.is_empty());
    }

    /// A recognizer with the given `supported_labels`, for testing
    /// [`effective_labels`](NerRecognizer::effective_labels) directly.
    fn recognizer_with(supported: Vec<LabelRef>) -> NerRecognizer {
        NerRecognizer::builder()
            .with_name("test")
            .with_mock_backend()
            .with_supported_labels(supported)
            .build()
            .expect("builder succeeds")
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
}
