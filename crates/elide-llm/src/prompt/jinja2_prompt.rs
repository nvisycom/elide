//! [`Jinja2Prompt`]: render prompt wording from a Jinja2 template file.
//!
//! Prompt-as-data: the user-prompt wording lives in a `.j2` template, so
//! users swap wording by editing a file rather than writing Rust. The
//! response *shape* is fixed per modality (the candidate batch the backend
//! extracts) and the source payload is attached by the backend, so the
//! template controls wording only.
//!
//! # Template variables
//!
//! - `hints`: list of `{ name, kind, value, snippet }` (text) or
//!   `{ name, kind, bbox: { x, y, width, height } }` (image)
//! - `labels`: document context labels
//! - `target_labels`: the entity types to emit
//! - `text`: the source text (text modality only)

use std::fs;
use std::marker::PhantomData;
use std::ops::Range;
use std::path::Path;

use elide_core::modality::Modality;
use elide_core::modality::image::{Image, ImageData};
use elide_core::modality::text::{Text, TextData};
use elide_core::recognition::RecognizerContext;
use elide_core::{Error, ErrorKind, Result};
use minijinja::{Environment, context};

use super::Prompt;

/// Half-width of the snippet rendered around a hint's location.
const HINT_SNIPPET_HALF_WIDTH: usize = 80;

/// Jinja2-template-driven [`Prompt`] impl.
///
/// Construct via [`from_file`](Self::from_file) or
/// [`from_template`](Self::from_template); the modality `M` selects which
/// template variables are populated.
pub struct Jinja2Prompt<M> {
    env: Environment<'static>,
    _modality: PhantomData<fn() -> M>,
}

impl<M> Jinja2Prompt<M> {
    /// Compile a prompt from a Jinja2 template string.
    ///
    /// # Errors
    ///
    /// Returns a validation error when the template fails to compile.
    pub fn from_template(template: impl Into<String>) -> Result<Self> {
        let mut env = Environment::new();
        env.add_template_owned("prompt", template.into())
            .map_err(|e| {
                Error::new(
                    ErrorKind::Validation,
                    format!("template compile error: {e}"),
                )
            })?;
        Ok(Self {
            env,
            _modality: PhantomData,
        })
    }

    /// Load and compile a prompt from a Jinja2 template file.
    ///
    /// # Errors
    ///
    /// Returns a validation error when the file is missing or the template
    /// fails to compile.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let raw = fs::read_to_string(path.as_ref())
            .map_err(|e| Error::new(ErrorKind::Validation, format!("reading prompt file: {e}")))?;
        Self::from_template(raw)
    }

    fn render(&self, ctx: minijinja::Value) -> String {
        self.env
            .get_template("prompt")
            .and_then(|t| t.render(ctx))
            .unwrap_or_default()
    }
}

impl Prompt<Text> for Jinja2Prompt<Text> {
    fn build(&self, data: &TextData, ctx: &RecognizerContext<'_, Text>) -> String {
        let text = data.text.as_str();
        let hints: Vec<_> = ctx
            .inclusions()
            .iter()
            .map(|h| {
                let range = h.location.start..h.location.end;
                let value = value_at(text, range.clone());
                let snippet = snippet_around(text, range);
                context! {
                    name => h.name.as_deref().unwrap_or(""),
                    kind => h.label.as_ref().map(|l| l.as_str().to_owned()).unwrap_or_else(|| "unknown".to_owned()),
                    value => value,
                    snippet => snippet,
                }
            })
            .collect();
        self.render(context! {
            text => text,
            hints => hints,
            labels => ctx.labels().to_vec(),
            target_labels => target_label_names(ctx),
        })
    }
}

impl Prompt<Image> for Jinja2Prompt<Image> {
    fn build(&self, _data: &ImageData, ctx: &RecognizerContext<'_, Image>) -> String {
        let hints: Vec<_> = ctx
            .inclusions()
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
        self.render(context! {
            hints => hints,
            labels => ctx.labels().to_vec(),
            target_labels => target_label_names(ctx),
        })
    }
}

fn value_at(text: &str, range: Range<usize>) -> &str {
    let Range { start, end } = range;
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

fn snippet_around(text: &str, range: Range<usize>) -> &str {
    let lo = floor_char_boundary(text, range.start.saturating_sub(HINT_SNIPPET_HALF_WIDTH));
    let hi = ceil_char_boundary(text, (range.end + HINT_SNIPPET_HALF_WIDTH).min(text.len()));
    &text[lo..hi]
}

/// The catalog's target labels as plain names, for the `{{ target_labels
/// }}` template variable.
fn target_label_names<M: Modality>(ctx: &RecognizerContext<'_, M>) -> Vec<String> {
    ctx.target_labels()
        .iter()
        .map(|l| l.as_str().to_owned())
        .collect()
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
