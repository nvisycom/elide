//! Recognizer layer: the LLM-driven [`LlmRecognizer`].
//!
//! `LlmRecognizer<B>` composes an [`LlmBackend`](crate::backend::LlmBackend) `B`
//! with a [`Prompt`](crate::prompt::Prompt) for the modality `B` serves (see
//! [`crate::prompt`]); the recognizer holds an `Arc<dyn Prompt<B::Modality>>` and
//! dispatches through it.

mod llm_recognizer;

pub use self::llm_recognizer::LlmRecognizer;
