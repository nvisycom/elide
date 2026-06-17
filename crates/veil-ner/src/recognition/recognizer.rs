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
//! [`Recognizer<Text>`]: veil_core::recognition::Recognizer

use std::sync::Arc;

use derive_builder::Builder;
use veil_core::entity::{Entity, LabelRef};
use veil_core::modality::text::{Text, TextLocation};
use veil_core::primitive::Confidence;
use veil_core::provenance::{Event, ModelEvent};
use veil_core::recognition::{Recognizer, RecognizerId, RecognizerInput, RecognizerOutput};
use veil_core::{Error, Result};

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
    /// Set via [`with_engine`], which accepts any concrete
    /// [`NerBackend`] impl by value and wraps it in `Arc` internally.
    ///
    /// [`with_engine`]: NerRecognizerBuilder::with_engine
    #[builder(setter(custom))]
    engine: Arc<dyn NerBackend>,
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
    /// Start the chainable builder. `name` and `engine` are
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
    /// Set the [`NerBackend`] backend that powers this recognizer.
    /// Accepts any concrete impl by value and wraps it in `Arc`.
    /// Required: `build` errors when this hasn't been called.
    #[must_use]
    pub fn with_engine<E: NerBackend>(mut self, engine: E) -> Self {
        self.engine = Some(Arc::new(engine));
        self
    }

    /// Finish the builder. Errors when `name` or `engine` is unset.
    pub fn build(self) -> Result<NerRecognizer> {
        self.try_build()
    }
}

impl Recognizer<Text> for NerRecognizer {
    fn id(&self) -> RecognizerId {
        RecognizerId::new(self.name.clone(), env!("CARGO_PKG_VERSION"))
    }

    async fn recognize(&self, input: &RecognizerInput<Text>) -> Result<RecognizerOutput<Text>> {
        let supported_borrowed: Vec<&str> = self
            .supported_labels
            .iter()
            .map(LabelRef::as_str)
            .collect();
        let labels = if supported_borrowed.is_empty() {
            None
        } else {
            Some(supported_borrowed.as_slice())
        };
        let request = NerRequest {
            text: input.content.text.as_str(),
            labels,
            language: input.language.as_ref(),
            correlation_id: input.correlation_id,
        };
        let response = self.engine.recognize(request).await?;

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
        Ok(RecognizerOutput::new(entities))
    }
}

#[cfg(test)]
mod tests {
    use veil_core::entity::{LabelRef, builtins};
    use veil_core::modality::text::TextData;

    use super::*;
    use crate::backend::NoopBackend;

    #[tokio::test]
    async fn noop_engine_yields_no_entities() {
        let rec = NerRecognizer::builder()
            .with_name("test")
            .with_engine(NoopBackend)
            .with_supported_labels(vec![
                LabelRef::from(&*builtins::PERSON_NAME),
                LabelRef::from(&*builtins::EMAIL_ADDRESS),
            ])
            .build()
            .expect("builder succeeds");
        let input = RecognizerInput::new(TextData::new("Alice Smith".to_owned()));
        let out = rec.recognize(&input).await.unwrap();
        assert!(out.entities.is_empty());
    }

    #[tokio::test]
    async fn empty_supported_labels_passes_none_to_engine() {
        let rec = NerRecognizer::builder()
            .with_name("test")
            .with_engine(NoopBackend)
            .build()
            .expect("builder succeeds");
        let input = RecognizerInput::new(TextData::new("Alice Smith".to_owned()));
        let out = rec.recognize(&input).await.unwrap();
        assert!(out.entities.is_empty());
    }
}
