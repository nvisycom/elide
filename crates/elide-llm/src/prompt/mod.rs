//! [`Prompt`]: the per-modality prompt wording consumed by
//! [`LlmRecognizer`].
//!
//! One trait per modality (`Prompt<Text>`, `Prompt<Image>`): the
//! recognizer holds `Arc<dyn Prompt<M>>` and renders the user prompt with
//! it. [`DefaultPrompt`] is the shipped impl covering both modalities;
//! users wanting different wording implement [`Prompt<M>`] and pass it to
//! [`LlmRecognizerBuilder::with_prompt`]. The response *shape* is fixed
//! per modality (the candidate batch the backend extracts), not chosen by
//! the prompt — so a prompt varies wording only.
//!
//! [`LlmRecognizer`]: crate::LlmRecognizer
//! [`LlmRecognizerBuilder::with_prompt`]: crate::recognition::LlmRecognizerBuilder::with_prompt

use elide_core::modality::Modality;
use elide_core::recognition::RecognizerContext;

mod default_prompt;
mod image_prompt;
mod text_prompt;

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
    /// Render the user prompt for `data` in `ctx`. Fold in hints, labels,
    /// and any instruction the model needs; the source payload (text,
    /// image bytes) is attached to the provider message by the backend.
    fn build(&self, data: &M::Data, ctx: &RecognizerContext<'_, M>) -> String;
}
