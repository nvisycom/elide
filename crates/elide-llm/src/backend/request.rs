//! [`LlmRequest`]: per-call input to an [`LlmBackend`].
//!
//! [`LlmBackend`]: super::LlmBackend

/// One per-call LLM request handed to a [`LlmBackend`].
///
/// [`LlmBackend`]: super::LlmBackend
#[derive(Debug, Clone)]
pub struct LlmRequest<'a> {
    /// Fully-rendered user prompt. The recognizer is responsible for
    /// folding the source text, hints, labels, and any base64-encoded
    /// binary payloads (images, audio) into this string.
    pub prompt: &'a str,
    /// Optional JSON schema the backend asks the model to constrain
    /// output against. Backends that support structured output
    /// (rig's `output_schema`) use it; backends that don't, ignore
    /// it. `None` means the recognizer's prompt is responsible for
    /// describing the expected output shape inline.
    pub schema: Option<&'a schemars::Schema>,
}
