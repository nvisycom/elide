//! [`RigBackend`]: rig-backed [`LlmBackend`].
//!
//! Wraps one of the four supported rig providers (OpenAI, Anthropic,
//! Gemini, Ollama) behind the modality-agnostic [`LlmBackend`]
//! surface. Owns its [`UsageTracker`] and [`LlmConfig`].
//!
//! [`LlmBackend`]: crate::backend::LlmBackend

mod config;
mod context;
mod inner;
mod usage;

use std::borrow::Cow;

use rig::agent::{Agent, AgentBuilder};
use rig::client::CompletionClient;
use rig::completion::{AssistantContent, Completion, CompletionModel, Message};
use veil_core::{Error as CoreError, ErrorKind as CoreErrorKind, Result};

pub use self::config::LlmConfig;
pub use self::context::ContextWindow;
use self::inner::{RigInner, dispatch};
pub use self::usage::{UsageStats, UsageTracker};
use super::http::{HttpConfig, build_http_client};
use super::{LlmBackend, LlmRequest, LlmResponse};
use crate::error::Error;
use crate::provider::LlmProvider;

const TARGET: &str = "veil_llm::backend::rig";

/// Rig-backed LLM backend.
///
/// Construct via [`builder`]. Owns the provider-specific rig agent
/// (created at build time), a [`UsageTracker`] that accumulates
/// per-call token usage, and the [`LlmConfig`] driving sampling +
/// compaction policy.
///
/// [`builder`]: Self::builder
pub struct RigBackend {
    agent: RigInner,
    config: LlmConfig,
    tracker: UsageTracker,
    model_name: String,
}

impl RigBackend {
    /// Start the chainable builder.
    #[must_use]
    pub fn builder() -> RigBackendBuilder {
        RigBackendBuilder::default()
    }

    /// Snapshot the cumulative token-usage counters.
    #[must_use]
    pub fn usage(&self) -> UsageStats {
        self.tracker.snapshot()
    }

    /// Reset the usage counters to zero.
    pub fn reset_usage(&self) {
        self.tracker.reset();
    }

    /// If `compact` is enabled, a context window is configured, and
    /// `prompt` exceeds the input budget, summarise it via an extra
    /// LLM call so it fits. Otherwise return the prompt unchanged.
    async fn maybe_compact<'a>(&self, prompt: &'a str) -> Result<Cow<'a, str>> {
        let Some(cw) = self.config.context_window.as_ref() else {
            return Ok(Cow::Borrowed(prompt));
        };
        if !self.config.compact || cw.fits(prompt) {
            return Ok(Cow::Borrowed(prompt));
        }

        let budget = cw.input_budget();
        tracing::info!(
            target: TARGET,
            prompt_len = prompt.len(),
            budget,
            "prompt exceeds input budget, compacting"
        );

        let compact_prompt = format!(
            "Summarize the following text so it fits within {budget} tokens. \
             Preserve all key facts and details.\n\n{prompt}"
        );
        let text = self.complete_text(&compact_prompt, None).await?;
        Ok(Cow::Owned(text))
    }

    /// Send a completion request and return the model's text reply.
    /// `schema`, when `Some`, is passed through to rig's
    /// `output_schema` so providers constrain the response.
    async fn complete_text(
        &self,
        prompt: &str,
        schema: Option<&schemars::Schema>,
    ) -> Result<String> {
        let (text, usage) = dispatch!(&self.agent, |agent| {
            let mut builder = agent
                .completion(prompt, Vec::<Message>::new())
                .await
                .map_err(Error::from)
                .map_err(crate::error::convert)?;
            if let Some(schema) = schema {
                builder = builder.output_schema(schema.clone());
            }
            let response = builder
                .send()
                .await
                .map_err(Error::from)
                .map_err(crate::error::convert)?;
            let text = extract_text(response.choice.iter())?;
            Ok::<_, Error>((text, response.usage))
        })?;
        self.tracker.record(&usage, 0);
        Ok(text)
    }
}

#[async_trait::async_trait]
impl LlmBackend for RigBackend {
    #[tracing::instrument(target = TARGET, skip_all, fields(model = %self.model_name))]
    async fn predict(&self, request: LlmRequest<'_>) -> Result<LlmResponse> {
        let prompt = self.maybe_compact(request.prompt).await?;
        let text = self.complete_text(&prompt, request.schema).await?;
        Ok(LlmResponse::new(text))
    }

    fn model(&self) -> &str {
        &self.model_name
    }
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
        })
        .map_err(|e| Error::Request(e.to_string()))
        .map_err(crate::error::convert)?;

        let preamble = config.preamble.as_deref();
        let agent = match &provider {
            #[cfg(feature = "openai-gpt")]
            LlmProvider::OpenAi(p) => {
                let client = p.openai_client(http).map_err(crate::error::convert)?;
                let model = client.completions_api().completion_model(p.model.as_str());
                RigInner::OpenAi(build_agent(model, &config, preamble))
            }
            #[cfg(feature = "anthropic-claude")]
            LlmProvider::Anthropic(p) => {
                let client = p.anthropic_client(http).map_err(crate::error::convert)?;
                let model = client.completion_model(p.model.as_str());
                RigInner::Anthropic(build_agent(model, &config, preamble))
            }
            #[cfg(feature = "google-gemini")]
            LlmProvider::Gemini(p) => {
                let client = p.gemini_client(http).map_err(crate::error::convert)?;
                let model = client.completion_model(p.model.as_str());
                RigInner::Gemini(build_agent(model, &config, preamble))
            }
            LlmProvider::Ollama(p) => {
                let client = p.ollama_client(http).map_err(crate::error::convert)?;
                let model = client.completion_model(p.model.as_str());
                RigInner::Ollama(build_agent(model, &config, preamble))
            }
        };

        Ok(RigBackend {
            agent,
            config,
            tracker: UsageTracker::new(),
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

fn extract_text<'a>(choices: impl Iterator<Item = &'a AssistantContent>) -> Result<String> {
    let texts: Vec<&str> = choices
        .filter_map(|c| match c {
            AssistantContent::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();

    if texts.is_empty() {
        return Err(CoreError::new(
            CoreErrorKind::Recognition,
            "LLM response contained no text content",
        ));
    }
    Ok(texts.join("\n"))
}
