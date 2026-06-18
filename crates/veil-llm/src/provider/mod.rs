//! Shared LLM provider connection parameters.
//!
//! The rig completion backend ([`backend::rig`]) consumes the
//! [`LlmProvider`] enum to dispatch to OpenAI / Anthropic / Google /
//! Ollama clients. Connection parameters (API key, model, optional base
//! URL) are split into [`AuthenticatedProvider`] +
//! [`UnauthenticatedProvider`] so providers that don't require a key
//! (Ollama) stay distinct in the type system.
//!
//! [`backend::rig`]: crate::backend

mod authenticated;
mod llm;
mod unauthenticated;

pub use self::authenticated::AuthenticatedProvider;
pub use self::llm::LlmProvider;
pub use self::unauthenticated::UnauthenticatedProvider;
