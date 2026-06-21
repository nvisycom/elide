//! Unified error type covering LLM provider, serialization, and tool failures.

use elide_core::{Error as CoreError, ErrorKind as CoreErrorKind};
use rig::completion::{CompletionError, PromptError, StructuredOutputError};
use rig::extractor::ExtractionError;
use rig::http_client::Error as HttpClientError;

/// Internal error type for LLM provider interactions.
///
/// Converted to [`CoreError`] at public API boundaries via the
/// [`convert`] helper.
///
/// [`CoreError`]: elide_core::Error
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

impl From<ExtractionError> for Error {
    fn from(err: ExtractionError) -> Self {
        match err {
            ExtractionError::NoData => {
                Self::Response("model extracted no structured data".to_string())
            }
            ExtractionError::DeserializationError(e) => Self::Json(e),
            ExtractionError::CompletionError(e) => Self::from(e),
        }
    }
}

/// Convert any error that can be turned into a provider [`Error`] into a
/// [`CoreError`]. Intended for `.map_err(crate::error::convert)` at
/// public API boundaries.
///
/// [`CoreError`]: elide_core::Error
pub(crate) fn convert<E: Into<Error>>(e: E) -> CoreError {
    CoreError::from(e.into())
}

impl From<Error> for CoreError {
    fn from(err: Error) -> Self {
        // Request errors are caller-side validation problems; every other
        // variant is a failure encountered while the recognizer drives the
        // model, so it maps to the recognition kind.
        let kind = match &err {
            Error::Request(_) => CoreErrorKind::Validation,
            Error::Http(_)
            | Error::Json(_)
            | Error::Provider(_)
            | Error::Response(_)
            | Error::Runtime(_) => CoreErrorKind::Recognition,
        };
        CoreError::new(kind, err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_error_maps_to_recognition() {
        let err = Error::Provider("Rate limit exceeded".to_string());
        let core_err: CoreError = err.into();
        assert_eq!(core_err.kind(), CoreErrorKind::Recognition);
    }

    #[test]
    fn request_error_maps_to_validation() {
        let err = Error::Request("bad URL".to_string());
        let core_err: CoreError = err.into();
        assert_eq!(core_err.kind(), CoreErrorKind::Validation);
    }
}
