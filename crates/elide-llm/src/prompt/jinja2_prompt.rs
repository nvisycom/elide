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
//! - `text`: the source text (text modality only)
//! - `language`: the call's primary language tag (`en`, `de-AT`), when known
//! - `target_labels`: the entity types to emit, in id order, each with an
//!   `id` (what the model must return as a detection's label), and a `name`
//!   and optional `description` localized in `language` (English fallback)
//! - `tags`: document classification tags (e.g. `medical`)

use std::fs;
use std::marker::PhantomData;
use std::path::Path;

use elide_core::modality::Modality;
use elide_core::modality::text::Text;
use elide_core::primitive::LanguageTag;
use elide_core::recognition::{Context, Subject};
use elide_core::{Error, Result};
use elide_image::modality::Image;
use minijinja::{Environment, Value, context};

use super::Prompt;

/// A [`Prompt`] whose wording comes from a caller-supplied Jinja2 template.
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
    /// A configuration error when the template fails to compile.
    pub fn from_template(template: impl Into<String>) -> Result<Self> {
        let mut env = Environment::new();
        env.add_template_owned("prompt", template.into())
            .map_err(|e| Error::configuration(format!("compiling prompt template: {e}")))?;
        Ok(Self {
            env,
            _modality: PhantomData,
        })
    }

    /// Load and compile a prompt from a Jinja2 template file.
    ///
    /// # Errors
    ///
    /// A configuration error when the file cannot be read or the template
    /// fails to compile.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let raw = fs::read_to_string(path.as_ref())
            .map_err(|e| Error::configuration(format!("reading prompt template: {e}")))?;
        Self::from_template(raw)
    }

    /// Render the template with `variables`, plus the variables every
    /// modality shares.
    fn render<N: Modality>(
        &self,
        subject: &Subject<N>,
        ctx: &Context<'_>,
        variables: Value,
    ) -> Result<String> {
        let language = ctx.languages(subject).primary();
        let shared = context! {
            language => language.map(LanguageTag::as_str),
            target_labels => target_labels(ctx, language),
            tags => ctx.tags().iter().map(|tag| tag.to_string()).collect::<Vec<_>>(),
        };
        self.env
            .get_template("prompt")
            .and_then(|template| template.render(context! { ..variables, ..shared }))
            .map_err(|e| Error::configuration(format!("rendering prompt template: {e}")))
    }
}

impl Prompt<Text> for Jinja2Prompt<Text> {
    fn build(&self, subject: &Subject<Text>, ctx: &Context<'_>) -> Result<String> {
        self.render(
            subject,
            ctx,
            context! { text => subject.data().text.as_str() },
        )
    }
}

impl Prompt<Image> for Jinja2Prompt<Image> {
    fn build(&self, subject: &Subject<Image>, ctx: &Context<'_>) -> Result<String> {
        self.render(subject, ctx, context! {})
    }
}

/// The catalog's labels for the `target_labels` variable, each an `id`,
/// `name`, and `description` localized in `language` (English fallback).
fn target_labels(ctx: &Context<'_>, language: Option<&LanguageTag>) -> Vec<Value> {
    let language = language.cloned().unwrap_or_else(LanguageTag::english);
    ctx.target_label_defs()
        .iter()
        .map(|label| {
            context! {
                id => label.id(),
                name => label.name(&language),
                description => label.description(&language),
            }
        })
        .collect()
}
