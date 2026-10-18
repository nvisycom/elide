//! LLM providers that do not require an API key.

use rig::providers::ollama::{Ollama, OllamaConfig};
use rig_reqwest::ReqwestMiddlewareClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Provider that does not require an API key (Ollama).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UnauthenticatedProvider {
    /// Model name (e.g. `llama3.1:8b`).
    pub model: String,
    /// Optional base URL override. `None` uses the provider's default
    /// (Ollama: `http://localhost:11434`).
    pub base_url: Option<String>,
}

impl UnauthenticatedProvider {
    /// Build an Ollama client sending through `http`.
    pub(crate) fn ollama_client(&self, http: ReqwestMiddlewareClient) -> Ollama {
        let mut config = OllamaConfig::new();
        if let Some(url) = &self.base_url {
            config = config.with_base_url(url);
        }
        config.connect(http)
    }
}
