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
//! - `tags`: document classification tags (e.g. `medical`)
//! - `target_labels`: the entity types to emit
//! - `text`: the source text (text modality only)

use std::fs;
use std::marker::PhantomData;
use std::path::Path;

use elide_core::modality::text::Text;
use elide_core::recognition::{Context, Subject};
use elide_core::{Error, Result};
use elide_image::modality::Image;
use minijinja::{Environment, context};

use super::Prompt;

/// Jinja2-template-driven [`Prompt`] impl.
///
/// Construct via [`from_file`] or [`from_template`]; the modality `M`
/// selects which template variables are populated.
///
/// [`from_file`]: Self::from_file
/// [`from_template`]: Self::from_template
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
            .map_err(|e| Error::configuration(format!("template compile error: {e}")))?;
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
            .map_err(|e| Error::configuration(format!("reading prompt file: {e}")))?;
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
    fn build(&self, subject: &Subject<Text>, ctx: &Context<'_>) -> String {
        self.render(context! {
            text => subject.data().text.as_str(),
            tags => tag_names(ctx),
            target_labels => target_label_names(ctx),
        })
    }
}

impl Prompt<Image> for Jinja2Prompt<Image> {
    fn build(&self, _subject: &Subject<Image>, ctx: &Context<'_>) -> String {
        self.render(context! {
            tags => tag_names(ctx),
            target_labels => target_label_names(ctx),
        })
    }
}

/// The catalog's target label ids, for the `{{ target_labels }}` template
/// variable. Ids (not localized names) so a custom template keys on the
/// stable identifier the model must return; the template author controls
/// their own wording and localization.
fn target_label_names(ctx: &Context<'_>) -> Vec<String> {
    ctx.target_labels()
        .iter()
        .map(|l| l.as_str().to_owned())
        .collect()
}

/// The context's tags as owned strings, the shape a template value takes
/// (minijinja builds a `Value` from `String`, not from the `HipStr` the context
/// stores them in).
fn tag_names(ctx: &Context<'_>) -> Vec<String> {
    ctx.tags().iter().map(|t| t.as_str().to_owned()).collect()
}
