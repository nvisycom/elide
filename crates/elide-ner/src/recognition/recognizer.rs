//! [`NerRecognizer`]: unified NER recognizer that drives any
//! [`NerBackend`] backend.
//!
//! Holds an `Arc<dyn NerBackend>` plus a [`NerModel`] (label-map +
//! ignore set + low-score demotion knobs) plus the recognizer's
//! advertised [`supported_kinds`]. On each `recognize` call it asks
//! the backend for spans (passing `Some(&supported_kinds)` when
//! non-empty for zero-shot backends, `None` when empty for
//! fixed-label backends), then normalizes through the model and
//! emits entities.
//!
//! Implements [`Recognizer<Text>`] so it composes with the
//! rest of the platform through the same trait every other text
//! recognizer uses.
//!
//! [`supported_kinds`]: NerRecognizer::supported_kinds
//! [`Recognizer<Text>`]: elide_core::recognition::Recognizer

use std::sync::Arc;

use derive_builder::Builder;
use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::primitive::Confidence;
use elide_core::provenance::{Event, ModelEvent};
use elide_core::recognition::{Recognizer, RecognizerContext, RecognizerId, RecognizerLanguage};
use elide_core::{Error, Result};

use super::config::NerModel;
use crate::backend::{NerBackend, NerRequest, RawNerSpan};

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
    /// emitted entity.
    name: String,
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
    /// Normalization knobs applied to the backend's raw output
    /// before entities are emitted.
    #[builder(default)]
    model: NerModel,
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

    /// Borrow the normalization config.
    #[must_use]
    pub fn model(&self) -> &NerModel {
        &self.model
    }

    fn build_entity(&self, span: &RawNerSpan, label: LabelRef) -> Entity<Text> {
        let raw_confidence = Confidence::clamped(span.score as f32);
        let confidence = if self.model.low_score_labels.contains(label.as_str()) {
            let demoted = f64::from(raw_confidence.get()) * self.model.low_score_multiplier;
            Confidence::clamped(demoted as f32)
        } else {
            raw_confidence
        };
        let location = TextLocation::new(span.offset.start, span.offset.end);
        let reason = format!("recognizer `{}` identified {}", self.name, label.as_str());
        let event = Event::model(
            "ner",
            confidence,
            location.clone(),
            ModelEvent {
                name: self.name.clone().into(),
                ..ModelEvent::default()
            },
        )
        .with_reason(reason);
        Entity::builder()
            .with_label(label)
            .with_location(location)
            .with_confidence(confidence)
            .with_event(event)
            .build()
            .expect("required fields provided")
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
        self.with_backend(crate::backend::MockBackend)
    }

    /// Finish the builder. Errors when `name` or `backend` is unset.
    pub fn build(self) -> Result<NerRecognizer> {
        self.try_build()
    }
}

impl Recognizer<Text> for NerRecognizer {
    fn id(&self) -> RecognizerId {
        RecognizerId::new(self.name.clone(), env!("CARGO_PKG_VERSION"))
    }

    async fn recognize(
        &self,
        data: &TextData,
        ctx: &RecognizerContext<Text>,
    ) -> Result<Vec<Entity<Text>>> {
        let supported_borrowed: Vec<&str> =
            self.supported_labels.iter().map(LabelRef::as_str).collect();
        let labels = if supported_borrowed.is_empty() {
            None
        } else {
            Some(supported_borrowed.as_slice())
        };
        let request = NerRequest {
            text: data.text.as_str(),
            labels,
            language: ctx.primary_language(),
            correlation_id: ctx.correlation_id,
        };
        let response = self.backend.recognize(request).await?;

        let entities: Vec<Entity<Text>> = response
            .spans
            .iter()
            .filter(|s| !self.model.labels_to_ignore.contains(s.label.as_str()))
            .filter_map(|s| {
                self.model
                    .label_map
                    .get(&s.label)
                    .filter(|name| {
                        self.supported_labels.is_empty()
                            || self.supported_labels.iter().any(|sl| sl == *name)
                    })
                    .cloned()
                    .map(|name| self.build_entity(s, name))
            })
            .collect();
        Ok(entities)
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::builtins;

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
        let ctx = RecognizerContext::new();
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
        let ctx = RecognizerContext::new();
        let out = rec.recognize(&data, &ctx).await.unwrap();
        assert!(out.is_empty());
    }
}
