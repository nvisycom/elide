//! [`Prompt`]: the per-modality prompt wording consumed by
//! [`LlmRecognizer`].
//!
//! One trait per modality (`Prompt<Text>`, `Prompt<Image>`): the
//! recognizer holds `Arc<dyn Prompt<M>>` and renders the user prompt with
//! it. [`DefaultPrompt`] is the shipped impl covering both modalities;
//! users wanting different wording implement [`Prompt<M>`] and pass it to
//! [`LlmRecognizerBuilder::with_prompt`]. The response *shape* is fixed
//! per modality (the candidate batch the backend extracts), not chosen by
//! the prompt, so a prompt varies wording only.
//!
//! [`LlmRecognizer`]: crate::LlmRecognizer
//! [`LlmRecognizerBuilder::with_prompt`]: crate::recognition::LlmRecognizerBuilder::with_prompt

use elide_core::entity::Label;
use elide_core::modality::Modality;
use elide_core::primitive::LanguageTag;
use elide_core::recognition::RecognizerContext;

mod default_prompt;
mod image_prompt;
mod text_prompt;

/// The target-label instruction shared by the text and image prompts.
///
/// Renders each label as `id (Localized name): Localized description.` in
/// `language` (English fallback), telling the model to return the stable
/// **id** as each detection's label while the localized name and
/// description guide what to find. Empty `labels` yields an empty string
/// (no restriction line).
fn target_labels_block(labels: &[Label], language: Option<&LanguageTag>) -> String {
    if labels.is_empty() {
        return String::new();
    }
    // Fall back to English when the call has no known language.
    let lang = language.cloned().unwrap_or_else(LanguageTag::english);
    let mut block = String::from(
        "\n\nEmit only these entity types. Return the id (before the \
         parenthesis) as each detection's `label`:",
    );
    for label in labels {
        match label.description(&lang) {
            Some(desc) => {
                block.push_str(&format!(
                    "\n- {} ({}): {}",
                    label.id(),
                    label.name(&lang),
                    desc
                ));
            }
            None => block.push_str(&format!("\n- {} ({})", label.id(), label.name(&lang))),
        }
    }
    block
}

#[cfg(feature = "jinja2")]
mod jinja2_prompt;

pub use self::default_prompt::DefaultPrompt;
#[cfg(feature = "jinja2")]
#[cfg_attr(docsrs, doc(cfg(feature = "jinja2")))]
pub use self::jinja2_prompt::Jinja2Prompt;

/// The per-modality prompt wording.
///
/// Renders the user prompt for one modality's payload (`data`) plus its
/// [`RecognizerContext<'_, M>`]. Wording only: the response shape and how
/// candidates become entities are not the prompt's concern.
pub trait Prompt<M>: Send + Sync + 'static
where
    M: Modality,
{
    /// Render the user prompt for `data` in `ctx`. Fold in hints, tags,
    /// and any instruction the model needs; the source payload (text,
    /// image bytes) is attached to the provider message by the backend.
    fn build(&self, data: &M::Data, ctx: &RecognizerContext<'_, M>) -> String;
}
