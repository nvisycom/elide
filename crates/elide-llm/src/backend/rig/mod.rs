//! [`RigBackend`]: rig-backed [`LlmBackend`].
//!
//! Wraps one of the four supported rig providers (OpenAI, Anthropic,
//! Gemini, Ollama) behind the modality-agnostic [`LlmBackend`] surface.
//!
//! [`LlmBackend`]: crate::backend::LlmBackend

mod config;
mod dispatch;

use elide_core::Result;
use elide_core::modality::image::{Image, ImageData};
use elide_core::modality::text::Text;
#[cfg(feature = "usage")]
use elide_core::recognition::TokenCounts;
use rig::ExtractionResponse;
use rig::client::CompletionClient;
use rig::completion::{Message, Usage};
use rig::extractor::ExtractorBuilder;
use rig::message::{ImageMediaType, UserContent};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use self::config::RigConfig;
use self::dispatch::{RigModel, dispatch};
use super::http::{HttpConfig, build_http_client};
use super::{LlmBackend, LlmRequest, LlmResponse};
use crate::error::Error;
use crate::modality::LlmModality;
use crate::provider::Provider;

const TARGET: &str = "elide_llm::backend::rig";

/// Rig-backed LLM backend.
///
/// Construct with [`new`] (default config) or [`new_with_config`]. Owns the
/// provider-specific rig agent (created at construction) and the
/// [`RigConfig`] driving sampling.
///
/// [`new`]: Self::new
/// [`new_with_config`]: Self::new_with_config
pub struct RigBackend {
    model: RigModel,
    config: RigConfig,
    model_name: String,
}

impl RigBackend {
    /// Build a backend for `provider` with the default [`RigConfig`].
    ///
    /// # Errors
    ///
    /// Returns the underlying rig / HTTP error when client construction
    /// fails.
    pub fn new(provider: Provider) -> Result<Self> {
        Self::new_with_config(provider, RigConfig::default())
    }

    /// Build a backend for `provider` with an explicit [`RigConfig`].
    ///
    /// The config is consumed here: it shapes the HTTP retry policy and the
    /// rig agent's sampling and preamble, all fixed at construction.
    ///
    /// # Errors
    ///
    /// Returns the underlying rig / HTTP error when client construction
    /// fails.
    pub fn new_with_config(provider: Provider, config: RigConfig) -> Result<Self> {
        let http = build_http_client(&HttpConfig {
            max_retries: config.max_retries,
            ..HttpConfig::default()
        })?;

        let model = match &provider {
            #[cfg(feature = "openai-gpt")]
            Provider::OpenAi(p) => {
                let client = p.openai_client(http)?;
                let model = client.completions_api().completion_model(p.model.as_str());
                RigModel::OpenAi(model)
            }
            #[cfg(feature = "anthropic-claude")]
            Provider::Anthropic(p) => {
                let client = p.anthropic_client(http)?;
                RigModel::Anthropic(client.completion_model(p.model.as_str()))
            }
            #[cfg(feature = "google-gemini")]
            Provider::Gemini(p) => {
                let client = p.gemini_client(http)?;
                RigModel::Gemini(client.completion_model(p.model.as_str()))
            }
            Provider::Ollama(p) => {
                let client = p.ollama_client(http)?;
                RigModel::Ollama(client.completion_model(p.model.as_str()))
            }
        };

        let model_name = provider.model().to_owned();
        Ok(RigBackend {
            model,
            config,
            model_name,
        })
    }

    /// Extract a structured candidate batch `T` from `message` using rig's
    /// [`Extractor`], built from this backend's provider model, returning `T`
    /// alongside the call's token [`Usage`]. The extractor constrains the
    /// model to `T`'s schema and parses the reply internally.
    ///
    /// [`Extractor`]: rig::extractor::Extractor
    /// [`Usage`]: rig::completion::Usage
    async fn extract_batch<T>(&self, message: Message) -> Result<(T, Usage), Error>
    where
        T: JsonSchema + for<'a> Deserialize<'a> + Serialize + Send + Sync + 'static,
    {
        let preamble = self.config.preamble.clone();
        let max_tokens = self.config.max_tokens;
        // `ExtractorBuilder` has no `temperature` setter, so pass it through
        // `additional_params` — rig merges these into the provider request.
        let params = serde_json::json!({ "temperature": self.config.temperature });
        dispatch!(&self.model, |model| {
            let mut builder = ExtractorBuilder::<T>::new(model.clone())
                .max_tokens(max_tokens)
                .additional_params(params);
            if let Some(p) = preamble.as_deref() {
                builder = builder.preamble(p);
            }
            let ExtractionResponse { data, usage } =
                builder.build().extract_with_usage(message).await?;
            Ok((data, usage))
        })
    }
}

/// Attach the call's token [`Usage`] to `response` under the `usage` feature;
/// without it, `usage` is unused and the response passes through.
///
/// [`Usage`]: rig::completion::Usage
#[cfg(feature = "usage")]
fn with_usage<M: LlmModality>(response: LlmResponse<M>, usage: Usage) -> LlmResponse<M> {
    // rig reports plain `u64`s, using `0` for "the provider gave no count".
    let present = |n: u64| (n != 0).then_some(n);
    response.with_tokens(TokenCounts {
        input: present(usage.input_tokens),
        output: present(usage.output_tokens),
        total: present(usage.total_tokens),
    })
}

/// Without the `usage` feature there is nothing to attach: the response passes
/// through and the call's [`Usage`] is dropped.
///
/// [`Usage`]: rig::completion::Usage
#[cfg(not(feature = "usage"))]
fn with_usage<M: LlmModality>(response: LlmResponse<M>, _usage: Usage) -> LlmResponse<M> {
    response
}

#[async_trait::async_trait]
impl LlmBackend<Text> for RigBackend {
    #[tracing::instrument(target = TARGET, skip_all, fields(model = %self.model_name))]
    async fn extract(&self, request: LlmRequest<'_, Text>) -> Result<LlmResponse<Text>> {
        let (candidates, usage) = self.extract_batch(Message::user(request.prompt)).await?;
        Ok(with_usage(LlmResponse::new(candidates), usage))
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
        let (candidates, usage) = self.extract_batch(message).await?;
        Ok(with_usage(LlmResponse::new(candidates), usage))
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
    let content = vec![
        UserContent::text(prompt),
        UserContent::image_raw(data.bytes.to_vec(), media_type, None),
    ];
    Message::User { content }
}
