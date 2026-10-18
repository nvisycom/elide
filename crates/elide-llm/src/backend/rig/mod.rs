//! [`RigBackend`]: rig-backed LLM backend.
//!
//! Wraps one of the four supported rig providers (OpenAI, Anthropic,
//! Gemini, Ollama) behind the modality-agnostic LLM backend surface.

mod config;

use std::marker::PhantomData;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use elide_core::Result;
use elide_core::backend::{Backend, TokenCounts};
use elide_core::entity::audit::ModelEvent;
use elide_core::modality::text::Text;
use elide_image::modality::{Image, ImageData, ImageFormat};
use rig::completion::{Message, Usage};
use rig::extractor::ExtractorBuilder;
use rig::message::{ImageMediaType, UserContent};
use rig::operation::Completion;
use rig::{DynModel, TypedPromptResponse};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use self::config::RigConfig;
use super::http::{HttpConfig, build_http_client};
use super::{LlmRequest, LlmResponse};
use crate::error::Error;
use crate::provider::Provider;

const TARGET: &str = "elide_llm::backend::rig";

/// Rig-backed LLM backend.
///
/// Construct with [`new`] (default config) or [`new_with_config`]. Owns the
/// provider's completion model (type-erased to [`DynModel`] at construction, so
/// one field serves every provider) and the [`RigConfig`] driving sampling.
///
/// [`new`]: Self::new
/// [`new_with_config`]: Self::new_with_config
pub struct RigBackend<M> {
    model: DynModel<Completion>,
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
    /// extractor's sampling and preamble, all fixed at construction.
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

        // Each provider's `.completion(model)` yields a `Model<Wire>`; erase it
        // to one `DynModel<Completion>` so the backend holds a single field
        // regardless of provider.
        let model: DynModel<Completion> = match &provider {
            #[cfg(feature = "openai-gpt")]
            Provider::OpenAi(p) => p.openai_client(http).completion(p.model.as_str()).into(),
            #[cfg(feature = "anthropic-claude")]
            Provider::Anthropic(p) => p.anthropic_client(http).completion(p.model.as_str()).into(),
            #[cfg(feature = "google-gemini")]
            Provider::Gemini(p) => p.gemini_client(http).completion(p.model.as_str()).into(),
            Provider::Ollama(p) => p.ollama_client(http).completion(p.model.as_str()).into(),
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
    /// Returns the batch alongside the provider-reported [`TokenCounts`] for the
    /// call (accumulated across rig's internal retries), so a `Metered` wrapper
    /// can bill it.
    ///
    /// [`Extractor`]: rig::extractor::Extractor
    async fn extract_batch<T>(&self, message: Message) -> Result<(T, TokenCounts), Error>
    where
        T: JsonSchema + for<'a> Deserialize<'a> + Serialize + Send + Sync + 'static,
    {
        // `ExtractorBuilder` has no `temperature` setter, so pass it through
        // `additional_params`; rig merges these into the provider request.
        let params = serde_json::json!({ "temperature": self.config.temperature });
        let mut builder = ExtractorBuilder::<T>::new(self.model.clone())
            .max_tokens(self.config.max_tokens)
            .additional_params(params);
        if let Some(p) = self.config.preamble.as_deref() {
            builder = builder.append_preamble(p);
        }
        let response: TypedPromptResponse<T> = builder.build().extract(message).await?;
        Ok((response.output, token_counts(&response.usage)))
    }
}

/// Map rig's completion [`Usage`] onto our [`TokenCounts`].
///
/// Both sides carry each count as `Option<u64>`, with the same meaning: `None`
/// is "the provider did not report this", distinct from `Some(0)`. So the fields
/// pass straight through — a `None` stays `None` rather than becoming a false,
/// billable-looking zero. `cached_input_tokens` is part of, not additional to,
/// the input count, matching [`TokenCounts::cached`].
fn token_counts(usage: &Usage) -> TokenCounts {
    TokenCounts {
        input: usage.input_tokens,
        output: usage.output_tokens,
        total: usage.total_tokens,
        cached: usage.cached_input_tokens,
        reasoning: usage.reasoning_tokens,
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
        let (candidates, tokens) = self.extract_batch(Message::user(request.prompt)).await?;
        Ok(LlmResponse::new(candidates).with_tokens(tokens))
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
        let (candidates, tokens) = self.extract_batch(message).await?;
        Ok(LlmResponse::new(candidates).with_tokens(tokens))
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
