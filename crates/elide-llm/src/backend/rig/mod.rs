//! [`RigBackend`]: rig-backed [`LlmBackend`].
//!
//! Wraps one of the four supported rig providers (OpenAI, Anthropic,
//! Gemini, Ollama) behind the modality-agnostic [`LlmBackend`] surface.
//!
//! [`LlmBackend`]: crate::backend::LlmBackend

mod config;
mod inner;

use elide_core::modality::image::{Image, ImageData};
use elide_core::modality::text::Text;
use elide_core::{Error as CoreError, ErrorKind as CoreErrorKind, Result};
use rig::OneOrMany;
use rig::agent::{Agent, AgentBuilder};
use rig::client::CompletionClient;
use rig::completion::{CompletionModel, Message};
use rig::extractor::ExtractorBuilder;
use rig::message::{ImageMediaType, UserContent};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use self::config::LlmConfig;
use self::inner::{RigInner, dispatch};
use super::http::{HttpConfig, build_http_client};
use super::{LlmBackend, LlmRequest, LlmResponse};
use crate::error::Error;
use crate::provider::LlmProvider;

const TARGET: &str = "elide_llm::backend::rig";

/// Rig-backed LLM backend.
///
/// Construct via [`builder`]. Owns the provider-specific rig agent
/// (created at build time) and the [`LlmConfig`] driving sampling.
///
/// [`builder`]: Self::builder
pub struct RigBackend {
    agent: RigInner,
    config: LlmConfig,
    model_name: String,
}

impl RigBackend {
    /// Start the chainable builder.
    #[must_use]
    pub fn builder() -> RigBackendBuilder {
        RigBackendBuilder::default()
    }

    /// Extract a structured candidate batch `T` from `message` using rig's
    /// [`Extractor`], built from this backend's provider model. The
    /// extractor constrains the model to `T`'s schema and parses the reply
    /// internally.
    ///
    /// [`Extractor`]: rig::extractor::Extractor
    async fn extract_batch<T>(&self, message: Message) -> Result<T, Error>
    where
        T: JsonSchema + for<'a> Deserialize<'a> + Serialize + Send + Sync + 'static,
    {
        let preamble = self.config.preamble.clone();
        dispatch!(&self.agent, |agent| {
            let mut builder = ExtractorBuilder::<_, T>::new((*agent.model).clone());
            if let Some(p) = preamble.as_deref() {
                builder = builder.preamble(p);
            }
            Ok(builder.build().extract(message).await?)
        })
    }
}

#[async_trait::async_trait]
impl LlmBackend<Text> for RigBackend {
    #[tracing::instrument(target = TARGET, skip_all, fields(model = %self.model_name))]
    async fn extract(&self, request: LlmRequest<'_, Text>) -> Result<LlmResponse<Text>> {
        let candidates = self.extract_batch(Message::user(request.prompt)).await?;
        Ok(LlmResponse::new(candidates))
    }

    fn model(&self) -> &str {
        &self.model_name
    }
}

#[async_trait::async_trait]
impl LlmBackend<Image> for RigBackend {
    #[tracing::instrument(target = TARGET, skip_all, fields(model = %self.model_name))]
    async fn extract(&self, request: LlmRequest<'_, Image>) -> Result<LlmResponse<Image>> {
        let message = image_message(request.prompt, request.data);
        let candidates = self.extract_batch(message).await?;
        Ok(LlmResponse::new(candidates))
    }

    fn model(&self) -> &str {
        &self.model_name
    }
}

/// Build a multimodal user [`Message`] carrying the prompt wording plus the
/// source image as a proper image content block.
fn image_message(prompt: &str, data: &ImageData) -> Message {
    let media_type = match data.extension() {
        "jpg" | "jpeg" => Some(ImageMediaType::JPEG),
        "png" => Some(ImageMediaType::PNG),
        "gif" => Some(ImageMediaType::GIF),
        "webp" => Some(ImageMediaType::WEBP),
        _ => None,
    };
    let content = OneOrMany::many([
        UserContent::text(prompt),
        UserContent::image_raw(data.bytes.to_vec(), media_type, None),
    ])
    .expect("two content items");
    Message::User { content }
}

/// Builder for [`RigBackend`].
#[derive(Debug, Default)]
pub struct RigBackendBuilder {
    provider: Option<LlmProvider>,
    config: Option<LlmConfig>,
}

impl RigBackendBuilder {
    /// Set the LLM provider. Required.
    #[must_use]
    pub fn with_provider(mut self, provider: LlmProvider) -> Self {
        self.provider = Some(provider);
        self
    }

    /// Set the agent config. Defaults to [`LlmConfig::default`].
    #[must_use]
    pub fn with_config(mut self, config: LlmConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Build the backend.
    ///
    /// # Errors
    ///
    /// Returns a validation error when `provider` is unset, and the
    /// underlying rig / HTTP error when client construction fails.
    pub fn build(self) -> Result<RigBackend> {
        let provider = self.provider.ok_or_else(|| {
            CoreError::new(
                CoreErrorKind::Validation,
                "RigBackendBuilder requires a provider",
            )
        })?;
        let config = self.config.unwrap_or_default();

        let http = build_http_client(&HttpConfig {
            max_retries: config.max_retries,
            ..HttpConfig::default()
        })?;

        let preamble = config.preamble.as_deref();
        let agent = match &provider {
            #[cfg(feature = "openai-gpt")]
            LlmProvider::OpenAi(p) => {
                let client = p.openai_client(http)?;
                let model = client.completions_api().completion_model(p.model.as_str());
                RigInner::OpenAi(build_agent(model, &config, preamble))
            }
            #[cfg(feature = "anthropic-claude")]
            LlmProvider::Anthropic(p) => {
                let client = p.anthropic_client(http)?;
                let model = client.completion_model(p.model.as_str());
                RigInner::Anthropic(build_agent(model, &config, preamble))
            }
            #[cfg(feature = "google-gemini")]
            LlmProvider::Gemini(p) => {
                let client = p.gemini_client(http)?;
                let model = client.completion_model(p.model.as_str());
                RigInner::Gemini(build_agent(model, &config, preamble))
            }
            LlmProvider::Ollama(p) => {
                let client = p.ollama_client(http)?;
                let model = client.completion_model(p.model.as_str());
                RigInner::Ollama(build_agent(model, &config, preamble))
            }
        };

        Ok(RigBackend {
            agent,
            config,
            model_name: provider.model().to_owned(),
        })
    }
}

fn build_agent<M: CompletionModel>(
    model: M,
    config: &LlmConfig,
    preamble: Option<&str>,
) -> Agent<M> {
    let mut b = AgentBuilder::new(model)
        .temperature(config.temperature)
        .max_tokens(config.max_tokens);
    if let Some(p) = preamble {
        b = b.preamble(p);
    }
    b.build()
}
