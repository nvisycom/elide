//! [`Prompt`]: pluggable per-modality prompt builder + response
//! lifter consumed by [`LlmRecognizer`].
//!
//! One trait per modality (`Prompt<Text>`, `Prompt<Image>`) — the
//! recognizer holds `Arc<dyn Prompt<M>>` and dispatches through it.
//! [`DefaultPrompt`] is the shipped impl, covering both modalities;
//! users wanting different wording, different response shapes, or
//! different localization policy implement [`Prompt<M>`] themselves
//! and pass their impl to
//! [`LlmRecognizerBuilder::with_prompt`].
//!
//! [`LlmRecognizer`]: super::LlmRecognizer
//! [`LlmRecognizerBuilder::with_prompt`]: super::LlmRecognizerBuilder::with_prompt

use nvisy_core::entity::Entity;
use nvisy_core::modality::Modality;
use nvisy_core::recognition::RecognizerInput;

use crate::backend::LlmResponse;

/// Pluggable prompt builder + response lifter for one modality.
///
/// Implementors own both halves of the modality-specific work: turn
/// a [`RecognizerInput<M>`] into the prompt string the backend
/// receives, then turn the backend's reply into entities. Keeping
/// both halves on one trait means the lifter has access to whatever
/// state the builder stamped into the prompt (label maps, hint
/// indices, etc.) by construction.
pub trait Prompt<M>: Send + Sync + 'static
where
    M: Modality,
{
    /// Render the user prompt for `input`. Fold in source data,
    /// hints, labels, and any base64-encoded binary payloads
    /// (images, audio) the model needs to see.
    fn build(&self, input: &RecognizerInput<M>) -> String;

    /// Optional JSON schema describing the expected response shape.
    /// When `Some`, the backend asks the model to constrain its
    /// reply (via rig's `output_schema` or whatever the backend
    /// uses). When `None`, the prompt itself is responsible for
    /// describing the expected shape inline.
    fn schema(&self) -> Option<&schemars::Schema> {
        None
    }

    /// Parse the response text into entities. The recognizer wraps
    /// these into a [`RecognizerOutput`].
    ///
    /// [`RecognizerOutput`]: nvisy_core::recognition::RecognizerOutput
    fn lift(&self, response: &LlmResponse, input: &RecognizerInput<M>) -> Vec<Entity<M>>;
}
