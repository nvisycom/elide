//! The [`Anonymizer`]: the redaction side of a modality's stage, mirroring the
//! Rust [`Anonymizer`](elide::redaction::Anonymizer) builder.
//!
//! A caller builds one per modality — [`Anonymizer::text`], [`Anonymizer::image`],
//! … — folds in [`Rule`]s, and hands it to
//! [`Orchestrator::with`](crate::orchestrator::Orchestrator::with) alongside the
//! matching analyzer. With no rules added, the stage uses a readable default
//! policy. An operator that does not apply to the modality (a black box on text)
//! is rejected — and, in TypeScript, is a compile-time error.

mod default;

use elide::entity::LabelRef;
use elide::modality::audio::Audio;
use elide::modality::image::Image;
use elide::modality::tabular::Tabular;
use elide::modality::text::Text;
use elide::redaction::Anonymizer as RustAnonymizer;
use wasm_bindgen::prelude::*;

use crate::analyzer::StageModality;
use crate::error::ElideError;
use crate::rule::{Rule, RuleKind};

/// The redaction side of a modality's stage: a sequence of rules, fixed to one
/// modality.
///
/// Mirrors the Rust [`Anonymizer`](elide::redaction::Anonymizer) builder. Built
/// with [`Anonymizer::text`] / [`Anonymizer::tabular`] / [`Anonymizer::image`] /
/// [`Anonymizer::audio`] and consumed by
/// [`Orchestrator::with`](crate::orchestrator::Orchestrator::with).
#[wasm_bindgen]
pub struct Anonymizer {
    modality: StageModality,
    rules: Vec<Rule>,
}

#[wasm_bindgen]
impl Anonymizer {
    /// An anonymizer for the text modality.
    #[wasm_bindgen(js_name = text)]
    pub fn text() -> Anonymizer {
        Anonymizer::new(StageModality::Text)
    }

    /// An anonymizer for the tabular modality (CSV cells).
    #[wasm_bindgen(js_name = tabular)]
    pub fn tabular() -> Anonymizer {
        Anonymizer::new(StageModality::Tabular)
    }

    /// An anonymizer for the image modality (redacted pixel regions).
    #[wasm_bindgen(js_name = image)]
    pub fn image() -> Anonymizer {
        Anonymizer::new(StageModality::Image)
    }

    /// An anonymizer for the audio modality (redacted time spans).
    #[wasm_bindgen(js_name = audio)]
    pub fn audio() -> Anonymizer {
        Anonymizer::new(StageModality::Audio)
    }

    /// Add a rule, in the order it should be tried. Consumes the rule. With no
    /// rules added, the stage uses the default redaction policy.
    #[wasm_bindgen(js_name = rule)]
    pub fn rule(mut self, rule: Rule) -> Anonymizer {
        self.rules.push(rule);
        self
    }
}

impl Anonymizer {
    fn new(modality: StageModality) -> Self {
        Self {
            modality,
            rules: Vec::new(),
        }
    }

    /// This anonymizer's modality.
    pub(crate) fn modality(&self) -> StageModality {
        self.modality
    }

    /// Build the Rust [`Anonymizer<Text>`](elide::redaction::Anonymizer).
    ///
    /// # Errors
    ///
    /// Errors if a rule's operator does not apply to text.
    pub(crate) fn build_text(self) -> Result<RustAnonymizer<Text>, ElideError> {
        if self.rules.is_empty() {
            return Ok(default::text());
        }
        let mut anonymizer = RustAnonymizer::new();
        for rule in self.rules {
            let (label, operator) = split(rule.0);
            anonymizer = anonymizer.with(operator.text_rule(label)?);
        }
        Ok(anonymizer)
    }

    /// Build the Rust [`Anonymizer<Tabular>`](elide::redaction::Anonymizer).
    ///
    /// # Errors
    ///
    /// Errors if a rule's operator does not apply to tabular.
    pub(crate) fn build_tabular(self) -> Result<RustAnonymizer<Tabular>, ElideError> {
        if self.rules.is_empty() {
            return Ok(default::tabular());
        }
        let mut anonymizer = RustAnonymizer::new();
        for rule in self.rules {
            let (label, operator) = split(rule.0);
            anonymizer = anonymizer.with(operator.tabular_rule(label)?);
        }
        Ok(anonymizer)
    }

    /// Build the Rust [`Anonymizer<Image>`](elide::redaction::Anonymizer) for the
    /// image's pixel regions.
    ///
    /// # Errors
    ///
    /// Errors if a rule's operator does not apply to image.
    pub(crate) fn build_image(self) -> Result<RustAnonymizer<Image>, ElideError> {
        if self.rules.is_empty() {
            return Ok(default::image_pixels());
        }
        let mut anonymizer = RustAnonymizer::new();
        for rule in self.rules {
            let (label, operator) = split(rule.0);
            anonymizer = anonymizer.with(operator.image_rule(label)?);
        }
        Ok(anonymizer)
    }

    /// Build the Rust [`Anonymizer<Audio>`](elide::redaction::Anonymizer).
    ///
    /// # Errors
    ///
    /// Errors if a rule's operator does not apply to audio.
    pub(crate) fn build_audio(self) -> Result<RustAnonymizer<Audio>, ElideError> {
        if self.rules.is_empty() {
            return Ok(default::audio());
        }
        let mut anonymizer = RustAnonymizer::new();
        for rule in self.rules {
            let (label, operator) = split(rule.0);
            anonymizer = anonymizer.with(operator.audio_rule(label)?);
        }
        Ok(anonymizer)
    }
}

/// Split a rule into its optional label reference and its operator.
fn split(kind: RuleKind) -> (Option<LabelRef>, crate::operator::OperatorKind) {
    match kind {
        RuleKind::Label { id, operator } => (Some(LabelRef::new(id)), operator),
        RuleKind::Fallback { operator } => (None, operator),
    }
}

pub(crate) use default::{
    audio as default_audio, image_metadata as default_image_metadata,
    image_pixels as default_image_pixels, tabular as default_text_tabular, text as default_text,
};
