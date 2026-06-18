//! Unified error type covering LLM provider, serialization, and tool failures.

use nvisy_core::{Error as CoreError, ErrorKind as CoreErrorKind};
use rig::completion::{CompletionError, PromptError, StructuredOutputError};
use rig::http_client::Error as HttpClientError;

/// Internal error type for LLM provider interactions.
///
/// Converted to [`CoreError`] at public API boundaries via the
/// [`convert`] helper.
///
/// [`CoreError`]: nvisy_core::Error
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    /// An HTTP / network error from the LLM provider.
    #[error("HTTP error: {0}")]
    Http(HttpClientError),

    /// A JSON (de)serialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// The LLM provider returned an error response.
    #[error("Provider error: {0}")]
    Provider(String),

    /// The LLM response was malformed or unexpected.
    #[error("Response error: {0}")]
    Response(String),

    /// A request construction or validation error.
    #[error("Request error: {0}")]
    Request(String),

    /// A runtime error (tool failure, agent limits, generation errors, etc.).
    #[error("{0}")]
    Runtime(String),
}

impl From<CompletionError> for Error {
    fn from(err: CompletionError) -> Self {
        match err {
            CompletionError::HttpError(e) => Self::Http(e),
            CompletionError::JsonError(e) => Self::Json(e),
            CompletionError::ProviderError(msg) => Self::Provider(msg),
            CompletionError::ResponseError(msg) => Self::Response(msg),
            CompletionError::RequestError(e) => Self::Request(e.to_string()),
            CompletionError::UrlError(e) => Self::Request(format!("URL: {e}")),
        }
    }
}

impl From<PromptError> for Error {
    fn from(err: PromptError) -> Self {
        match err {
            PromptError::CompletionError(e) => Self::from(e),
            PromptError::ToolError(e) => Self::Runtime(format!("tool: {e}")),
            PromptError::ToolServerError(e) => Self::Runtime(format!("tool server: {e}")),
            PromptError::MaxTurnsError { max_turns, .. } => {
                Self::Runtime(format!("agent exceeded max turn limit ({max_turns})"))
            }
            PromptError::PromptCancelled { reason, .. } => {
                Self::Runtime(format!("prompt cancelled: {reason}"))
            }
            PromptError::UnknownToolCall { tool_name, .. } => {
                Self::Runtime(format!("agent called unknown tool: {tool_name}"))
            }
        }
    }
}

impl From<StructuredOutputError> for Error {
    fn from(err: StructuredOutputError) -> Self {
        match err {
            StructuredOutputError::PromptError(e) => Self::from(*e),
            StructuredOutputError::DeserializationError(e) => {
                Self::Response(format!("structured output: {e}"))
            }
            StructuredOutputError::EmptyResponse => {
                Self::Response("model returned no content".to_string())
            }
        }
    }
}

/// Convert any error that can be turned into a provider [`Error`] into a
/// [`CoreError`]. Intended for `.map_err(crate::error::convert)` at
/// public API boundaries.
///
/// [`CoreError`]: nvisy_core::Error
pub(crate) fn convert<E: Into<Error>>(e: E) -> CoreError {
    CoreError::from(e.into())
}

impl From<Error> for CoreError {
    fn from(err: Error) -> Self {
        match &err {
            Error::Http(_) => CoreError::connection(err.to_string(), "rig", true),
            Error::Json(_) => {
                CoreError::new(CoreErrorKind::Serialization, err.to_string()).with_component("rig")
            }
            Error::Provider(msg) => {
                let retryable = is_retryable_provider_error(msg);
                CoreError::connection(err.to_string(), "rig", retryable)
            }
            Error::Response(_) => CoreError::runtime(err.to_string(), "rig", false),
            Error::Request(_) => CoreError::validation(err.to_string(), "rig"),
            Error::Runtime(_) => CoreError::runtime(err.to_string(), "rig", false),
        }
    }
}

/// Check if a provider error message indicates a retryable condition.
///
/// This uses substring matching against known error patterns from OpenAI,
/// Anthropic, and Google. If upstream providers change their error
/// message format this may need updating — the test suite below
/// validates all known patterns.
fn is_retryable_provider_error(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("rate_limit")
        || lower.contains("rate limit")
        || lower.contains("overloaded")
        || lower.contains("timeout")
        || lower.contains("429")
        || lower.contains("503")
        || lower.contains("529")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_patterns() {
        // OpenAI
        assert!(is_retryable_provider_error("Rate limit reached for gpt-4o"));
        assert!(is_retryable_provider_error(
            "Error code: 429 - You exceeded your current quota"
        ));

        // Anthropic
        assert!(is_retryable_provider_error("overloaded_error: Overloaded"));
        assert!(is_retryable_provider_error(
            "rate_limit_error: Rate limited"
        ));
        assert!(is_retryable_provider_error("Error code: 529 - Overloaded"));

        // Google
        assert!(is_retryable_provider_error("503 Service Unavailable"));
        assert!(is_retryable_provider_error("Request timeout"));
    }

    #[test]
    fn non_retryable_patterns() {
        assert!(!is_retryable_provider_error("invalid_api_key"));
        assert!(!is_retryable_provider_error("model not found"));
        assert!(!is_retryable_provider_error("content policy violation"));
        assert!(!is_retryable_provider_error(""));
    }

    #[test]
    fn provider_error_conversion_is_retryable() {
        let err = Error::Provider("Rate limit exceeded".to_string());
        let core_err: CoreError = err.into();
        assert!(core_err.is_retryable());
    }

    #[test]
    fn provider_error_conversion_is_not_retryable() {
        let err = Error::Provider("invalid model".to_string());
        let core_err: CoreError = err.into();
        assert!(!core_err.is_retryable());
    }
}
