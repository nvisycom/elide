//! [`RigBackend`]: rig-backed LLM backend.
//!
//! Wraps one of the four supported rig providers (OpenAI, Anthropic,
//! Gemini, Ollama) behind the modality-agnostic LLM backend surface.

mod config;
mod error;

use std::marker::PhantomData;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use elide_core::Result;
use elide_core::backend::Backend;
use elide_core::entity::audit::ModelId;
use elide_core::modality::text::Text;
use elide_image::modality::{Image, ImageData, ImageFormat};
use rig::completion::Message;
use rig::extractor::ExtractorBuilder;
use rig::message::{ImageMediaType, UserContent};
use rig::operation::Completion;
use rig::{AgentBuilder, DynModel, TypedPromptResponse};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use self::config::RigConfig;
use self::error::Error;
use super::http::http_client;
use super::{LlmRequest, LlmResponse};
use crate::provider::Provider;

const TARGET: &str = "elide_llm::backend::rig";

/// An [`LlmBackend`](crate::backend::LlmBackend) that calls a provider through the `rig` crate.
///
/// Owns the provider's completion model (type-erased to [`DynModel`] at
/// construction, so one field serves every provider) and the [`RigConfig`]
/// driving sampling.
pub struct RigBackend<M> {
    model: DynModel<Completion>,
    config: RigConfig,
    /// The model each response names.
    model_id: ModelId,
    _modality: PhantomData<fn() -> M>,
}

impl<M> RigBackend<M> {
    /// A backend calling `provider`, configured by `config`
    /// ([`RigConfig::default`] for the defaults).
    ///
    /// The config shapes the HTTP retry policy and the extractor's sampling
    /// and preamble, all fixed at construction.
    ///
    /// # Errors
    ///
    /// A configuration error when the HTTP client cannot be built.
    pub fn new(provider: Provider, config: RigConfig) -> Result<Self> {
        let http = http_client(config.max_retries)?;

        // Each provider's completion method yields a `Model<Wire>`; erase it to
        // one `DynModel<Completion>` so the backend holds a single field
        // regardless of provider. Ollama uses `native_completion` (`/api/chat`),
        // not `completion` (which rig 0.44 routes to the OpenAI-compatible
        // `/v1/chat/completions`), so a daemon exposing only its native endpoint
        // keeps working.
        let model: DynModel<Completion> = match &provider {
            #[cfg(feature = "openai-gpt")]
            Provider::OpenAi(p) => p.openai_client(http).completion(p.model.as_str()).into(),
            #[cfg(feature = "anthropic-claude")]
            Provider::Anthropic(p) => p.anthropic_client(http).completion(p.model.as_str()).into(),
            #[cfg(feature = "google-gemini")]
            Provider::Gemini(p) => p.gemini_client(http).completion(p.model.as_str()).into(),
            Provider::Ollama(p) => p
                .ollama_client(http)
                .native_completion(p.model.as_str())
                .into(),
        };

        let model_id = ModelId::new(provider.model().to_owned());
        Ok(RigBackend {
            model,
            config,
            model_id,
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
        // Set temperature on the agent builder so it rides rig's typed request
        // field, which each provider places in its own request shape (Gemini's
        // `generationConfig`, OpenAI's/Anthropic's top level). Passing it through
        // `additional_params` would put it at the request's top level, where
        // Gemini ignores it.
        let agent = AgentBuilder::new(self.model.clone()).temperature(self.config.temperature);
        let mut builder =
            ExtractorBuilder::<T>::from_agent_builder(agent).max_tokens(self.config.max_tokens);
        if let Some(p) = self.config.preamble.as_deref() {
            builder = builder.append_preamble(p);
        }
        let response: TypedPromptResponse<T> = builder.build().extract(message).await?;
        Ok(response.output)
    }
}

#[async_trait::async_trait]
impl Backend for RigBackend<Text> {
    type Request<'a> = LlmRequest<'a, Text>;
    type Response = LlmResponse<Text>;

    #[tracing::instrument(target = TARGET, skip_all, fields(model = %self.model_id.name, modality = "text"))]
    async fn call(&self, request: LlmRequest<'_, Text>) -> Result<LlmResponse<Text>> {
        let candidates = self.extract_batch(Message::user(request.prompt)).await?;
        Ok(LlmResponse::new(self.model_id.clone(), candidates))
    }
}

#[async_trait::async_trait]
impl Backend for RigBackend<Image> {
    type Request<'a> = LlmRequest<'a, Image>;
    type Response = LlmResponse<Image>;

    #[tracing::instrument(target = TARGET, skip_all, fields(model = %self.model_id.name, modality = "image"))]
    async fn call(&self, request: LlmRequest<'_, Image>) -> Result<LlmResponse<Image>> {
        let message = image_message(request.prompt, request.data)?;
        let candidates = self.extract_batch(message).await?;
        Ok(LlmResponse::new(self.model_id.clone(), candidates))
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
