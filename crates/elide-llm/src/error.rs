//! Internal error type bridging rig's provider errors to
//! [`elide_core::Error`].
//!
//! The crate exposes no error type of its own — every public result is an
//! [`elide_core::Error`]. This private [`Error`] exists only to host the
//! `From<rig error>` conversions the orphan rule forbids writing directly
//! onto `elide_core::Error`, and to keep call sites `?`-ergonomic. Its
//! variants name the [`ErrorKind`] each maps to; the underlying rig error
//! is preserved as the error source.
//!
//! [`ErrorKind`]: elide_core::ErrorKind

use elide_core::{Error as CoreError, ErrorKind};
use rig::completion::{CompletionError, PromptError, StructuredOutputError};
use rig::extractor::ExtractionError;
use rig::http_client::Error as HttpClientError;

/// Internal LLM error, tagged by the [`ErrorKind`] it maps to.
///
/// [`ErrorKind`]: elide_core::ErrorKind
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    /// A transport-layer failure reaching the provider (HTTP, network).
    #[error("transport: {0}")]
    Transport(Box<dyn std::error::Error + Send + Sync>),

    /// The provider returned an error response.
    #[error("provider: {0}")]
    Provider(Box<dyn std::error::Error + Send + Sync>),

    /// A malformed, empty, or unparseable model reply.
    #[error("response: {0}")]
    Response(Box<dyn std::error::Error + Send + Sync>),

    /// A request-construction / configuration problem.
    #[error("request: {0}")]
    Request(Box<dyn std::error::Error + Send + Sync>),
}

impl From<CompletionError> for Error {
    fn from(err: CompletionError) -> Self {
        match err {
            CompletionError::HttpError(_) | CompletionError::RequestError(_) => {
                Self::Transport(Box::new(err))
            }
            CompletionError::ProviderError(_) => Self::Provider(Box::new(err)),
            CompletionError::ResponseError(_) | CompletionError::JsonError(_) => {
                Self::Response(Box::new(err))
            }
            CompletionError::UrlError(_) => Self::Request(Box::new(err)),
        }
    }
}

impl From<PromptError> for Error {
    fn from(err: PromptError) -> Self {
        match err {
            PromptError::CompletionError(e) => Self::from(e),
            // Tool / turn-limit / cancellation failures occur while the
            // recognizer drives the model.
            other => Self::Response(Box::new(other)),
        }
    }
}

impl From<StructuredOutputError> for Error {
    fn from(err: StructuredOutputError) -> Self {
        match err {
            StructuredOutputError::PromptError(e) => Self::from(*e),
            other => Self::Response(Box::new(other)),
        }
    }
}

impl From<ExtractionError> for Error {
    fn from(err: ExtractionError) -> Self {
        match err {
            ExtractionError::CompletionError(e) => Self::from(e),
            // No data / deserialization failure: no usable structured reply.
            other => Self::Response(Box::new(other)),
        }
    }
}

impl From<HttpClientError> for Error {
    fn from(err: HttpClientError) -> Self {
        Self::Transport(Box::new(err))
    }
}

impl From<Error> for CoreError {
    fn from(err: Error) -> Self {
        let kind = match &err {
            Error::Transport(_) => ErrorKind::Transport,
            Error::Provider(_) => ErrorKind::Provider,
            // A bad reply is a recognition-time failure.
            Error::Response(_) => ErrorKind::Recognition,
            // Request construction is caller-side validation.
            Error::Request(_) => ErrorKind::Validation,
        };
        CoreError::new(kind, err)
    }
}
