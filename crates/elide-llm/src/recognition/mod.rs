//! Recognizer layer: the LLM-driven [`LlmRecognizer`].
//!
//! `LlmRecognizer<M>` composes a modality-agnostic [`LlmBackend`] with a
//! modality-specific [`Prompt<M>`] (see [`crate::prompt`]); the recognizer
//! holds an `Arc<dyn Prompt<M>>` and dispatches through it.
//!
//! [`LlmBackend`]: crate::backend::LlmBackend
//! [`Prompt<M>`]: crate::prompt::Prompt

mod llm_recognizer;

pub use self::llm_recognizer::{LlmRecognizer, LlmRecognizerBuilder};
