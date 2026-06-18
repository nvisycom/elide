//! Recognizer layer: the LLM-driven [`LlmRecognizer`].
//!
//! `LlmRecognizer<M>` composes a modality-agnostic [`LlmBackend`]
//! with a modality-specific [`Prompt<M>`]. [`DefaultPrompt`] is the
//! shipped impl, covering both text and image; users wanting
//! different prompt wording or different response handling
//! implement [`Prompt<M>`] and pass their impl through the builder.
//!
//! [`LlmBackend`]: crate::backend::LlmBackend

mod candidates;
mod default_prompt;
mod file_prompt;
mod lift;
mod llm_recognizer;
mod localize;
mod prompt;
mod response_parser;
mod schemas;
mod text_prompt;
mod vlm_prompt;

pub use self::default_prompt::DefaultPrompt;
pub use self::file_prompt::FilePrompt;
pub use self::llm_recognizer::{LlmRecognizer, LlmRecognizerBuilder};
pub use self::prompt::Prompt;
