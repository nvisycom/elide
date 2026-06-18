//! [`DefaultPrompt`]: the shipped [`Prompt`] impl, covering both
//! [`Text`] and [`Image`].
//!
//! Both impls follow the same pattern: build a structured-output
//! prompt with shared system instructions, ask the model to return
//! `{ "entities": [...] }`, deserialise into a candidate vec, and
//! lift each candidate into an `Entity<M>`. For text, the
//! candidate's `context` field is used to localize the value back
//! into a byte range; for image, the bounding box arrives in
//! normalised `[0, 1]` coordinates and is scaled to pixel space.
//!
//! No label-map or labels-to-ignore filtering is applied here — the
//! model's kinds pass through verbatim. Use [`FilePrompt`] when you
//! need to remap raw model labels.
//!
//! [`FilePrompt`]: super::file_prompt::FilePrompt

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use nvisy_core::entity::Entity;
use nvisy_core::modality::{Image, Text};
use nvisy_core::recognition::{LabelMap, RecognizerInput};
use schemars::Schema;

use super::candidates::{TextCandidates, VlmCandidates};
use super::lift::{lift_image, lift_text};
use super::prompt::Prompt;
use super::response_parser::parse_json;
use super::schemas::{text_schema, vlm_schema};
use super::text_prompt::TextPromptBuilder;
use super::vlm_prompt::VlmPromptBuilder;
use crate::backend::LlmResponse;

/// Shipped [`Prompt`] impl covering both [`Text`] and [`Image`].
///
/// Stateless zero-sized type. Customise behaviour by writing your
/// own [`Prompt<M>`] impl rather than tweaking this one.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultPrompt;

impl Prompt<Text> for DefaultPrompt {
    fn build(&self, input: &RecognizerInput<Text>) -> String {
        TextPromptBuilder::new(input.data.text.as_str(), &input.hints, &input.labels).build()
    }

    fn schema(&self) -> Option<&Schema> {
        Some(text_schema())
    }

    fn lift(&self, response: &LlmResponse, input: &RecognizerInput<Text>) -> Vec<Entity<Text>> {
        let Ok(parsed): Result<TextCandidates, _> = parse_json(&response.text) else {
            return Vec::new();
        };
        lift_text(input, parsed.entities, &LabelMap::new(), &[])
    }
}

impl Prompt<Image> for DefaultPrompt {
    fn build(&self, input: &RecognizerInput<Image>) -> String {
        let image_b64 = STANDARD.encode(input.data.bytes.as_ref());
        VlmPromptBuilder::new(&input.hints, &input.labels).build(&image_b64)
    }

    fn schema(&self) -> Option<&Schema> {
        Some(vlm_schema())
    }

    fn lift(&self, response: &LlmResponse, input: &RecognizerInput<Image>) -> Vec<Entity<Image>> {
        let Ok(parsed): Result<VlmCandidates, _> = parse_json(&response.text) else {
            return Vec::new();
        };
        lift_image(input, parsed.entities, &LabelMap::new(), &[])
    }
}
