//! LLM provider connection parameters.
//!
//! The [`RigBackend`] consumes the [`Provider`] enum to dispatch to
//! OpenAI / Anthropic / Google / Ollama clients. Connection parameters
//! (API key, model, optional base URL) are split into
//! [`AuthenticatedProvider`] + [`UnauthenticatedProvider`] so providers
//! that don't require a key (Ollama) stay distinct in the type system.
//!
//! The [`Provider`] enum holds connection parameters and the model name
//! only; the actual rig client is constructed when a [`RigBackend`] is
//! built.
//!
//! [`RigBackend`]: crate::backend::RigBackend

mod authenticated;
mod unauthenticated;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use self::authenticated::AuthenticatedProvider;
pub use self::unauthenticated::UnauthenticatedProvider;

/// Supported LLM providers for agent-based tasks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Provider {
    /// OpenAI (GPT-4o, GPT-4, etc.).
    #[cfg(feature = "openai-gpt")]
    #[cfg_attr(docsrs, doc(cfg(feature = "openai-gpt")))]
    OpenAi(AuthenticatedProvider),
    /// Anthropic (Claude).
    #[cfg(feature = "anthropic-claude")]
    #[cfg_attr(docsrs, doc(cfg(feature = "anthropic-claude")))]
    Anthropic(AuthenticatedProvider),
    /// Google Gemini.
    #[cfg(feature = "google-gemini")]
    #[cfg_attr(docsrs, doc(cfg(feature = "google-gemini")))]
    Gemini(AuthenticatedProvider),
    /// Ollama (local models).
    Ollama(UnauthenticatedProvider),
}

impl Provider {
    /// Create an OpenAI provider.
    #[cfg(feature = "openai-gpt")]
    #[cfg_attr(docsrs, doc(cfg(feature = "openai-gpt")))]
    pub fn openai(api_key: &str, model: &str) -> Self {
        Self::OpenAi(AuthenticatedProvider {
            api_key: api_key.to_owned(),
            model: model.to_owned(),
            base_url: None,
        })
    }

    /// Create an Anthropic provider.
    #[cfg(feature = "anthropic-claude")]
    #[cfg_attr(docsrs, doc(cfg(feature = "anthropic-claude")))]
    pub fn anthropic(api_key: &str, model: &str) -> Self {
        Self::Anthropic(AuthenticatedProvider {
            api_key: api_key.to_owned(),
            model: model.to_owned(),
            base_url: None,
        })
    }

    /// Create a Google Gemini provider.
    #[cfg(feature = "google-gemini")]
    #[cfg_attr(docsrs, doc(cfg(feature = "google-gemini")))]
    pub fn gemini(api_key: &str, model: &str) -> Self {
        Self::Gemini(AuthenticatedProvider {
            api_key: api_key.to_owned(),
            model: model.to_owned(),
            base_url: None,
        })
    }

    /// Create an Ollama provider using the default local URL.
    pub fn ollama(model: &str) -> Self {
        Self::Ollama(UnauthenticatedProvider {
            model: model.to_owned(),
            base_url: None,
        })
    }

    /// Create an Ollama provider with a custom base URL.
    pub fn ollama_with_url(model: &str, url: &str) -> Self {
        Self::Ollama(UnauthenticatedProvider {
            model: model.to_owned(),
            base_url: Some(url.to_owned()),
        })
    }

    /// The model name for this provider.
    pub fn model(&self) -> &str {
        match self {
            #[cfg(feature = "openai-gpt")]
            Self::OpenAi(p) => &p.model,
            #[cfg(feature = "anthropic-claude")]
            Self::Anthropic(p) => &p.model,
            #[cfg(feature = "google-gemini")]
            Self::Gemini(p) => &p.model,
            Self::Ollama(p) => &p.model,
        }
    }
}
