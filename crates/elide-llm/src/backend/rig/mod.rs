//! [`RigBackend`]: rig-backed [`LlmBackend`].
//!
//! Wraps one of the four supported rig providers (OpenAI, Anthropic,
//! Gemini, Ollama) behind the modality-agnostic [`LlmBackend`] surface.
//!
//! [`LlmBackend`]: crate::backend::LlmBackend

mod config;
mod dispatch;

use std::marker::PhantomData;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use elide_core::Result;
use elide_core::backend::Backend;
use elide_core::entity::audit::ModelEvent;
use elide_core::modality::text::Text;
use elide_image::modality::{Image, ImageData, ImageFormat};
use rig::client::CompletionClient;
use rig::completion::Message;
use rig::extractor::ExtractorBuilder;
use rig::message::{ImageMediaType, UserContent};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use self::config::RigConfig;
use self::dispatch::{RigModel, dispatch};
use super::http::{HttpConfig, build_http_client};
use super::{LlmRequest, LlmResponse};
use crate::error::Error;
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
pub struct RigBackend<M> {
    model: RigModel,
    config: RigConfig,
    model_name: String,
    _modality: PhantomData<fn() -> M>,
}

impl<M> RigBackend<M> {
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
            _modality: PhantomData,
        })
    }

    /// Extract a structured candidate batch `T` from `message` using rig's
    /// [`Extractor`], built from this backend's provider model. The extractor
    /// constrains the model to `T`'s schema and parses the reply internally.
    ///
    /// [`Extractor`]: rig::extractor::Extractor
    async fn extract_batch<T>(&self, message: Message) -> Result<T, Error>
    where
        T: JsonSchema + for<'a> Deserialize<'a> + Serialize + Send + Sync + 'static,
    {
        let preamble = self.config.preamble.clone();
        let max_tokens = self.config.max_tokens;
        // `ExtractorBuilder` has no `temperature` setter, so pass it through
        // `additional_params`, rig merges these into the provider request.
        let params = serde_json::json!({ "temperature": self.config.temperature });
        dispatch!(&self.model, |model| {
            let mut builder = ExtractorBuilder::<T>::new(model.clone())
                .max_tokens(max_tokens)
                .additional_params(params);
            if let Some(p) = preamble.as_deref() {
                builder = builder.preamble(p);
            }
            Ok(builder.build().extract(message).await?)
        })
    }
}

#[async_trait::async_trait]
impl Backend for RigBackend<Text> {
    type Request<'a> = LlmRequest<'a, Text>;
    type Response = LlmResponse<Text>;

    fn provenance(&self) -> ModelEvent {
        ModelEvent {
            name: self.model_name.clone().into(),
            ..ModelEvent::default()
        }
    }

    #[tracing::instrument(target = TARGET, skip_all, fields(model = %self.model_name))]
    async fn call(&self, request: LlmRequest<'_, Text>) -> Result<LlmResponse<Text>> {
        let candidates = self.extract_batch(Message::user(request.prompt)).await?;
        Ok(LlmResponse::new(candidates))
    }
}

#[async_trait::async_trait]
impl Backend for RigBackend<Image> {
    type Request<'a> = LlmRequest<'a, Image>;
    type Response = LlmResponse<Image>;

    fn provenance(&self) -> ModelEvent {
        ModelEvent {
            name: self.model_name.clone().into(),
            ..ModelEvent::default()
        }
    }

    #[tracing::instrument(target = TARGET, skip_all, fields(model = %self.model_name))]
    async fn call(&self, request: LlmRequest<'_, Image>) -> Result<LlmResponse<Image>> {
        let message = image_message(request.prompt, request.data)?;
        let candidates = self.extract_batch(message).await?;
        Ok(LlmResponse::new(candidates))
    }
}

/// Build a multimodal user [`Message`] carrying the prompt wording plus the
/// source image as a base64 PNG image content block.
///
/// The image is decoded and re-encoded to PNG regardless of its source format,
/// then base64-encoded: rig's providers reject a raw-bytes source and every
/// vision model accepts PNG, so this normalizes any input (including a TIFF, or
/// bytes with no filename to hint the format) to one the model takes. PNG is
/// lossless, so a detection request loses no detail to the transcode.
///
/// # Errors
///
/// [`ErrorKind::Processing`](elide_core::ErrorKind::Processing) if the PNG
/// re-encode fails.
fn image_message(prompt: &str, data: &ImageData) -> Result<Message> {
    // The payload is already decoded; re-encode it to PNG (the media type the
    // API takes) without re-opening the bytes.
    let png = data.image().encode_as(ImageFormat::Png)?;
    let encoded = BASE64.encode(&png);
    let content = vec![
        UserContent::text(prompt),
        UserContent::image_base64(encoded, Some(ImageMediaType::PNG), None),
    ];
    Ok(Message::User { content })
}
