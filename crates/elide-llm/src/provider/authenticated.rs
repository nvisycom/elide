//! LLM providers that require an API key.

use std::fmt;

#[cfg(feature = "anthropic-claude")]
use rig::providers::anthropic::{Anthropic, AnthropicConfig};
#[cfg(feature = "google-gemini")]
use rig::providers::gemini::{Gemini, GeminiConfig};
#[cfg(feature = "openai-gpt")]
use rig::providers::openai::{OpenAI, OpenAIConfig};
#[cfg(any(
    feature = "openai-gpt",
    feature = "anthropic-claude",
    feature = "google-gemini"
))]
use rig_reqwest::ReqwestMiddlewareClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Provider that requires an API key (OpenAI, Anthropic, Gemini).
///
/// Connection parameters used by the [`RigBackend`].
///
/// [`RigBackend`]: crate::backend::RigBackend
#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AuthenticatedProvider {
    /// API key for the provider.
    pub api_key: String,
    /// Model name (e.g. `gpt-4o`, `claude-3-5-sonnet-20241022`).
    pub model: String,
    /// Optional base URL override. `None` uses the provider's default.
    pub base_url: Option<String>,
}

impl fmt::Debug for AuthenticatedProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthenticatedProvider")
            .field("api_key", &"***")
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl AuthenticatedProvider {
    /// Build an OpenAI client sending through `http`.
    #[cfg(feature = "openai-gpt")]
    #[cfg_attr(docsrs, doc(cfg(feature = "openai-gpt")))]
    pub(crate) fn openai_client(&self, http: ReqwestMiddlewareClient) -> OpenAI {
        let mut config = OpenAIConfig::new(&self.api_key);
        if let Some(url) = &self.base_url {
            config = config.with_base_url(url);
        }
        config.connect(http)
    }

    /// Build a Gemini client sending through `http`.
    #[cfg(feature = "google-gemini")]
    #[cfg_attr(docsrs, doc(cfg(feature = "google-gemini")))]
    pub(crate) fn gemini_client(&self, http: ReqwestMiddlewareClient) -> Gemini {
        let mut config = GeminiConfig::new(&self.api_key);
        if let Some(url) = &self.base_url {
            config = config.with_base_url(url);
        }
        config.connect(http)
    }

    /// Build an Anthropic client sending through `http`.
    #[cfg(feature = "anthropic-claude")]
    #[cfg_attr(docsrs, doc(cfg(feature = "anthropic-claude")))]
    pub(crate) fn anthropic_client(&self, http: ReqwestMiddlewareClient) -> Anthropic {
        let mut config = AnthropicConfig::new(&self.api_key);
        if let Some(url) = &self.base_url {
            config = config.with_base_url(url);
        }
        config.connect(http)
    }
}
