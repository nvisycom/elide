//! [`LlmRequest`]: per-call input to an LLM backend.

use elide_core::backend::BackendRequest;
use elide_core::modality::Modality;
use elide_core::primitive::LanguageTag;

use crate::modality::LlmModality;

/// One per-call LLM request handed to an LLM backend, generic over
/// the modality.
///
/// Carries the fully-rendered prompt wording (produced by the recognizer's
/// [`Prompt`]) plus the source payload, so the backend can assemble the
/// provider message, folding in the image bytes for a multimodal call.
///
/// [`Prompt`]: crate::prompt::Prompt
#[derive(Debug, Clone, Copy)]
pub struct LlmRequest<'a, M: Modality> {
    /// Fully-rendered user prompt wording.
    pub prompt: &'a str,
    /// The source payload (text, image bytes, …) the backend folds into
    /// the provider message.
    pub data: &'a M::Data,
    /// The call's primary language, asserted or detected, for a
    /// language-routed backend.
    pub language: Option<&'a LanguageTag>,
}

impl<M: LlmModality> BackendRequest for LlmRequest<'_, M> {
    fn language(&self) -> Option<&LanguageTag> {
        self.language
    }
}
