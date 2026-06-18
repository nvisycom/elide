//! LLM providers that do not require an API key.

use reqwest_middleware::ClientWithMiddleware;
use rig::client::Nothing;
use rig::providers::ollama;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::Error;

/// Provider that does not require an API key (Ollama, future local
/// STT backends).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UnauthenticatedProvider {
    /// Model name (e.g. `llama3.1:8b`).
    pub model: String,
    /// Optional base URL override. `None` uses the provider's default
    /// (Ollama: `http://localhost:11434`).
    pub base_url: Option<String>,
}

impl UnauthenticatedProvider {
    /// Build an Ollama rig-core client.
    pub(crate) fn ollama_client(
        &self,
        http: ClientWithMiddleware,
    ) -> Result<ollama::Client<ClientWithMiddleware>, Error> {
        let mut b = ollama::Client::builder().api_key(Nothing).http_client(http);
        if let Some(url) = &self.base_url {
            b = b.base_url(url);
        }
        b.build().map_err(|e| Error::Request(e.to_string()))
    }
}
