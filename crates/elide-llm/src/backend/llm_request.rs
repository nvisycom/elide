//! [`LlmRequest`]: per-call input to an [`LlmBackend`].
//!
//! [`LlmBackend`]: super::LlmBackend

use elide_core::modality::Modality;

/// One per-call LLM request handed to an [`LlmBackend<M>`], generic over
/// the modality.
///
/// Carries the fully-rendered prompt wording (produced by the recognizer's
/// [`Prompt`]) plus the source payload, so the backend can assemble the
/// provider message — folding in the image bytes for a multimodal call.
///
/// [`LlmBackend<M>`]: super::LlmBackend
/// [`Prompt`]: crate::prompt::Prompt
#[derive(Debug, Clone, Copy)]
pub struct LlmRequest<'a, M: Modality> {
    /// Fully-rendered user prompt wording.
    pub prompt: &'a str,
    /// The source payload (text, image bytes, …) the backend folds into
    /// the provider message.
    pub data: &'a M::Data,
}

impl<'a, M: Modality> LlmRequest<'a, M> {
    /// Construct a request from rendered `prompt` wording and the source
    /// `data`.
    pub fn new(prompt: &'a str, data: &'a M::Data) -> Self {
        Self { prompt, data }
    }
}
