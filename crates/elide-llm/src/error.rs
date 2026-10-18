//! Internal error type bridging rig's provider errors to
//! [`elide_core::Error`].
//!
//! The crate exposes no error type of its own, every public result is an
//! [`elide_core::Error`]. This private [`Error`] exists only to host the
//! `From<rig error>` conversions the orphan rule forbids writing directly
//! onto `elide_core::Error`, and to keep call sites `?`-ergonomic. Its
//! variants name the [`ErrorKind`] each maps to; the underlying rig error
//! is preserved as the error source.
//!
//! [`ErrorKind`]: elide_core::ErrorKind

use elide_core::{Error as CoreError, ErrorKind};
use rig::completion::{PromptError, StructuredOutputError};
use rig::error::{ErrorKind as RigErrorKind, ProviderError};
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

    /// The provider's reply decoded but is not a usable answer.
    #[error("response: {0}")]
    Response(Box<dyn std::error::Error + Send + Sync>),

    /// Request construction failed (a bad URL or unsupported option).
    #[error("request: {0}")]
    Request(Box<dyn std::error::Error + Send + Sync>),
}

impl From<ProviderError> for Error {
    fn from(err: ProviderError) -> Self {
        match &err {
            // No reply reached us: a reset, timeout, or unreadable response.
            ProviderError::Http(_) => Self::Transport(Box::new(err)),
            // The provider answered with a failure (an error status/envelope,
            // rejected credentials, an expired cache handle).
            ProviderError::Provider(_)
            | ProviderError::ProviderResponse(_)
            | ProviderError::InvalidAuthentication(_)
            | ProviderError::CacheExpired { .. } => Self::Provider(Box::new(err)),
            // A reply arrived but is not a usable answer to the request.
            ProviderError::Response(_)
            | ProviderError::Json(_)
            | ProviderError::Truncated
            | ProviderError::MismatchedDimensions { .. } => Self::Response(Box::new(err)),
            // A failure a relay delivered as its report carries the original
            // kind; classify by that so a relayed transport failure keeps its
            // transport (retryable) classification rather than flattening to a
            // response error.
            ProviderError::Relayed(report) => match report.kind {
                RigErrorKind::Http | RigErrorKind::Timeout => Self::Transport(Box::new(err)),
                RigErrorKind::Provider | RigErrorKind::ProviderResponse => {
                    Self::Provider(Box::new(err))
                }
                RigErrorKind::Url | RigErrorKind::Request => Self::Request(Box::new(err)),
                _ => Self::Response(Box::new(err)),
            },
            // Caller-side: a request we could not build.
            ProviderError::Url(_)
            | ProviderError::Request(_)
            | ProviderError::UnsupportedOption(_) => Self::Request(Box::new(err)),
            // `ProviderError` is `#[non_exhaustive]`, so a `rig` upgrade can add
            // a variant the compiler won't flag here. The warn is the cue to
            // classify it above.
            _ => {
                tracing::warn!(error = %err, "unmapped rig ProviderError variant");
                Self::Response(Box::new(err))
            }
        }
    }
}

impl From<PromptError> for Error {
    fn from(err: PromptError) -> Self {
        match err {
            PromptError::Provider(e) => Self::from(e),
            // Memory / turn-limit / cancellation / tool failures occur while the
            // extractor drives the model; none is a provider reply.
            other => Self::Response(Box::new(other)),
        }
    }
}

impl From<StructuredOutputError> for Error {
    fn from(err: StructuredOutputError) -> Self {
        match err {
            StructuredOutputError::Prompt(e) => Self::from(e),
            // No usable structured reply: an undeserializable or empty output.
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
            // Request construction is caller-side configuration.
            Error::Request(_) => ErrorKind::Configuration,
        };
        CoreError::new(kind, err)
    }
}
