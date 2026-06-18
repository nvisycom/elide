//! [`FilePrompt`]: load a [`Prompt`] from a TOML file.
//!
//! Prompt-as-data shape: the user-prompt template plus the label
//! map plus the labels-to-ignore set all live in a single TOML
//! file. Users swap behaviour by editing the file, not by writing
//! Rust. Templates use Jinja2 syntax via `minijinja`.
//!
//! # TOML schema
//!
//! ```toml
//! schema_version = 1
//!
//! [meta]
//! name = "ner-default"
//! modality = "text"   # or "image"
//!
//! # Optional. Maps model-emitted labels to canonical entity
//! # labels. Use snake_case label names on the right-hand side.
//! [label_map]
//! person = "person_name"
//! email = "email_address"
//!
//! # Optional. Drop spans the model returned with any of these
//! # labels (case-sensitive).
//! labels_to_ignore = ["MISC", "O"]
//!
//! # Jinja2-syntax template. Available variables:
//! #   - text: source text (text modality only)
//! #   - image_b64: base64-encoded image bytes (image modality only)
//! #   - hints: list of { name, kind, value, snippet } (text) or
//! #            { name, kind, bbox: { x, y, width, height } } (image)
//! #   - labels: list of document context labels
//! template = """
//! Detect every sensitive entity in:
//! ---
//! {{ text }}
//! ---
//! {% if labels %}Labels: {{ labels | join(", ") }}{% endif %}
//! Return JSON: {"entities": [...]}.
//! """
//! ```

use std::collections::HashMap;
use std::fs;
use std::marker::PhantomData;
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::image::{Image, ImageData};
use elide_core::modality::text::{Text, TextData};
use elide_core::recognition::{LabelMap, RecognizerContext};
use elide_core::{Error, ErrorKind, Result};
use minijinja::{Environment, context};
use schemars::Schema;
use serde::Deserialize;

use super::candidates::{TextCandidates, VlmCandidates};
use super::lift::{lift_image, lift_text};
use super::prompt::Prompt;
use super::response_parser::parse_json;
use super::schemas::{text_schema, vlm_schema};
use crate::backend::LlmResponse;

/// Half-width of the snippet rendered around a hint's location.
const HINT_SNIPPET_HALF_WIDTH: usize = 80;

/// File-driven [`Prompt`] impl.
///
/// Construct via [`from_toml_file`] or [`from_toml`]; the modality
/// (`M`) is checked against `meta.modality` at parse time.
///
/// [`from_toml_file`]: Self::from_toml_file
/// [`from_toml`]: Self::from_toml
pub struct FilePrompt<M> {
    template: String,
    label_map: LabelMap,
    labels_to_ignore: Vec<String>,
    env: Environment<'static>,
    _modality: PhantomData<fn() -> M>,
}

#[derive(Debug, Deserialize)]
struct PromptFile {
    #[allow(dead_code)]
    schema_version: Option<u32>,
    meta: PromptMeta,
    #[serde(default)]
    label_map: Option<HashMap<String, String>>,
    #[serde(default)]
    labels_to_ignore: Vec<String>,
    template: String,
}

#[derive(Debug, Deserialize)]
struct PromptMeta {
    #[allow(dead_code)]
    name: Option<String>,
    modality: String,
}

impl<M> FilePrompt<M> {
    fn from_parsed(parsed: PromptFile, expected_modality: &str) -> Result<Self> {
        if parsed.meta.modality != expected_modality {
            return Err(Error::new(
                ErrorKind::Validation,
                format!(
                    "prompt file modality is {:?}, expected {:?}",
                    parsed.meta.modality, expected_modality
                ),
            ));
        }

        let mut label_map = LabelMap::new();
        if let Some(entries) = parsed.label_map {
            for (model_label, entity_label) in entries {
                label_map.insert(model_label, LabelRef::new(entity_label));
            }
        }

        let mut env = Environment::new();
        env.add_template_owned("prompt", parsed.template.clone())
            .map_err(|e| {
                Error::new(
                    ErrorKind::Validation,
                    format!("template compile error: {e}"),
                )
            })?;

        Ok(Self {
            template: parsed.template,
            label_map,
            labels_to_ignore: parsed.labels_to_ignore,
            env,
            _modality: PhantomData,
        })
    }

    /// Borrow the loaded template source.
    #[must_use]
    pub fn template(&self) -> &str {
        &self.template
    }

    /// Borrow the configured label map.
    #[must_use]
    pub fn label_map(&self) -> &LabelMap {
        &self.label_map
    }
}

impl FilePrompt<Text> {
    /// Load a text-modality prompt from a TOML file.
    ///
    /// # Errors
    ///
    /// Returns a validation error when the file is missing, malformed,
    /// declares a non-`text` modality, or contains an invalid Jinja2
    /// template.
    pub fn from_toml_file(path: impl AsRef<Path>) -> Result<Self> {
        let raw = fs::read_to_string(path.as_ref())
            .map_err(|e| Error::new(ErrorKind::Validation, format!("reading prompt file: {e}")))?;
        Self::from_toml(&raw)
    }

    /// Load a text-modality prompt from a TOML string.
    ///
    /// # Errors
    ///
    /// See [`from_toml_file`].
    ///
    /// [`from_toml_file`]: Self::from_toml_file
    pub fn from_toml(raw: &str) -> Result<Self> {
        let parsed: PromptFile = toml::from_str(raw)
            .map_err(|e| Error::new(ErrorKind::Validation, format!("parsing prompt TOML: {e}")))?;
        Self::from_parsed(parsed, "text")
    }
}

impl FilePrompt<Image> {
    /// Load an image-modality prompt from a TOML file.
    ///
    /// # Errors
    ///
    /// Same as the text-modality loader.
    pub fn from_toml_file(path: impl AsRef<Path>) -> Result<Self> {
        let raw = fs::read_to_string(path.as_ref())
            .map_err(|e| Error::new(ErrorKind::Validation, format!("reading prompt file: {e}")))?;
        Self::from_toml(&raw)
    }

    /// Load an image-modality prompt from a TOML string.
    ///
    /// # Errors
    ///
    /// Same as the text-modality loader.
    pub fn from_toml(raw: &str) -> Result<Self> {
        let parsed: PromptFile = toml::from_str(raw)
            .map_err(|e| Error::new(ErrorKind::Validation, format!("parsing prompt TOML: {e}")))?;
        Self::from_parsed(parsed, "image")
    }
}

impl Prompt<Text> for FilePrompt<Text> {
    fn build(&self, data: &TextData, ctx: &RecognizerContext<Text>) -> String {
        let text = data.text.as_str();
        let hints: Vec<_> = ctx
            .hints
            .iter()
            .map(|h| {
                let value = value_at(text, h.location.start, h.location.end);
                let snippet = snippet_around(text, h.location.start, h.location.end);
                context! {
                    name => h.name.as_deref().unwrap_or(""),
                    kind => h.label.as_ref().map(|l| l.as_str().to_owned()).unwrap_or_else(|| "unknown".to_owned()),
                    value => value,
                    snippet => snippet,
                }
            })
            .collect();
        let jinja_ctx = context! {
            text => text,
            hints => hints,
            labels => ctx.labels.clone(),
        };
        self.env
            .get_template("prompt")
            .and_then(|t| t.render(jinja_ctx))
            .unwrap_or_default()
    }

    fn schema(&self) -> Option<&Schema> {
        Some(text_schema())
    }

    fn lift(
        &self,
        response: &LlmResponse,
        data: &TextData,
        _ctx: &RecognizerContext<Text>,
    ) -> Vec<Entity<Text>> {
        let Ok(parsed): Result<TextCandidates, _> = parse_json(&response.text) else {
            return Vec::new();
        };
        lift_text(
            data,
            parsed.entities,
            &self.label_map,
            &self.labels_to_ignore,
        )
    }
}

impl Prompt<Image> for FilePrompt<Image> {
    fn build(&self, data: &ImageData, ctx: &RecognizerContext<Image>) -> String {
        let image_b64 = STANDARD.encode(data.bytes.as_ref());
        let hints: Vec<_> = ctx
            .hints
            .iter()
            .map(|h| {
                let bbox = &h.location.bounding_box;
                context! {
                    name => h.name.as_deref().unwrap_or(""),
                    kind => h.label.as_ref().map(|l| l.as_str().to_owned()).unwrap_or_else(|| "unknown".to_owned()),
                    bbox => context! {
                        x => bbox.min.x,
                        y => bbox.min.y,
                        width => bbox.width(),
                        height => bbox.height(),
                    },
                }
            })
            .collect();
        let jinja_ctx = context! {
            image_b64 => image_b64,
            hints => hints,
            labels => ctx.labels.clone(),
        };
        self.env
            .get_template("prompt")
            .and_then(|t| t.render(jinja_ctx))
            .unwrap_or_default()
    }

    fn schema(&self) -> Option<&Schema> {
        Some(vlm_schema())
    }

    fn lift(
        &self,
        response: &LlmResponse,
        data: &ImageData,
        _ctx: &RecognizerContext<Image>,
    ) -> Vec<Entity<Image>> {
        let Ok(parsed): Result<VlmCandidates, _> = parse_json(&response.text) else {
            return Vec::new();
        };
        lift_image(
            data,
            parsed.entities,
            &self.label_map,
            &self.labels_to_ignore,
        )
    }
}

fn value_at(text: &str, start: usize, end: usize) -> &str {
    if start < end
        && end <= text.len()
        && text.is_char_boundary(start)
        && text.is_char_boundary(end)
    {
        &text[start..end]
    } else {
        ""
    }
}

fn snippet_around(text: &str, start: usize, end: usize) -> &str {
    let lo = floor_char_boundary(text, start.saturating_sub(HINT_SNIPPET_HALF_WIDTH));
    let hi = ceil_char_boundary(text, (end + HINT_SNIPPET_HALF_WIDTH).min(text.len()));
    &text[lo..hi]
}

fn floor_char_boundary(s: &str, mut pos: usize) -> usize {
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

fn ceil_char_boundary(s: &str, mut pos: usize) -> usize {
    while pos < s.len() && !s.is_char_boundary(pos) {
        pos += 1;
    }
    pos
}
