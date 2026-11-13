//! The bridge from `rig`'s errors to [`elide_core::Error`].
//!
//! The crate exposes no error type of its own: every public result is an
//! [`elide_core::Error`]. This private [`Error`] hosts the `From<rig error>`
//! conversions the orphan rule forbids writing onto `elide_core::Error`
//! directly, and keeps call sites `?`-ergonomic. Its variants name the
//! [`ErrorKind`] each maps to.
//!
//! A model's reply echoes the document it was asked about, so it is the
//! document's PII. A failure about the reply is therefore rebuilt as a
//! [`ResponseFault`] that says what went wrong and where, never what the
//! reply said: rig keeps the raw reply on its errors, and `serde_json`'s
//! messages quote the values they failed on.

use std::fmt;

use elide_core::{Error as CoreError, ErrorKind};
use rig::completion::{PromptError, StructuredOutputError};
use rig::error::{ErrorKind as RigErrorKind, ErrorReport, ProviderError};
use rig::http_client::Error as HttpClientError;
use serde_json::error::Category;

const TARGET: &str = "elide_llm::backend::rig::error";

/// Internal LLM error, tagged by the [`ErrorKind`] it maps to.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    /// A transport-layer failure reaching the provider (HTTP, network).
    #[error("transport: {0}")]
    Transport(Box<dyn std::error::Error + Send + Sync>),

    /// The provider returned an error response.
    #[error("provider: {0}")]
    Provider(Box<dyn std::error::Error + Send + Sync>),

    /// The provider's reply arrived but is not a usable answer.
    #[error("response: {0}")]
    Response(ResponseFault),

    /// Request construction failed (a bad URL or unsupported option).
    #[error("request: {0}")]
    Request(Box<dyn std::error::Error + Send + Sync>),
}

/// Why a reply was unusable, described without its content.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ResponseFault {
    /// The reply is not the JSON the request asked for.
    #[error("the model's reply did not parse: {0}")]
    Unparsable(JsonFault),
    /// The reply could not be read as a provider response.
    #[error("the provider's reply was malformed")]
    Malformed,
    /// The reply was cut off before it was complete.
    #[error("the provider's reply was truncated")]
    Truncated,
    /// The model returned no structured output.
    #[error("the model returned no structured output")]
    Empty,
    /// The extraction run stopped before the model replied.
    #[error("the extraction run stopped: {0}")]
    Stopped(String),
    /// A relayed failure of a reply, with the code the provider gave.
    #[error("the provider's reply failed{}", code.as_deref().map(|c| format!(" (code {c})")).unwrap_or_default())]
    Relayed {
        /// The provider's machine code for the failure, when it gave one.
        code: Option<String>,
    },
    /// A failure `rig` reports that is not classified here.
    #[error("the provider's reply failed in an unclassified way")]
    Unclassified,
}

/// A JSON parse failure: what kind of failure and where, never the text.
#[derive(Debug)]
pub(crate) struct JsonFault {
    category: Category,
    line: usize,
    column: usize,
}

impl From<&serde_json::Error> for JsonFault {
    fn from(error: &serde_json::Error) -> Self {
        Self {
            category: error.classify(),
            line: error.line(),
            column: error.column(),
        }
    }
}

impl fmt::Display for JsonFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let category = match self.category {
            Category::Io => "an I/O error",
            Category::Syntax => "a syntax error",
            Category::Data => "a data error",
            Category::Eof => "an unexpected end of input",
        };
        write!(
            f,
            "{category} at line {}, column {}",
            self.line, self.column
        )
    }
}

impl From<ProviderError> for Error {
    fn from(err: ProviderError) -> Self {
        match err {
            // No reply reached us: a reset, timeout, or unreadable response.
            ProviderError::Http(_) => Self::Transport(Box::new(err)),
            // The provider answered with a failure (an error status/envelope,
            // rejected credentials, an expired cache handle). Its message is
            // about the request, not the document.
            ProviderError::Provider(_)
            | ProviderError::ProviderResponse(_)
            | ProviderError::InvalidAuthentication(_)
            | ProviderError::CacheExpired { .. } => Self::Provider(Box::new(err)),
            // A reply arrived but is not a usable answer to the request.
            ProviderError::Json(error) => {
                Self::Response(ResponseFault::Unparsable(JsonFault::from(&*error)))
            }
            ProviderError::Response(_) => Self::Response(ResponseFault::Malformed),
            ProviderError::Truncated => Self::Response(ResponseFault::Truncated),
            ProviderError::MismatchedDimensions { .. } => Self::Response(ResponseFault::Malformed),
            // A failure a relay delivered as its report carries the original
            // kind.
            ProviderError::Relayed(ref report) => Relayed::of(report).error(err),
            // Caller-side: a request we could not build.
            ProviderError::Url(_)
            | ProviderError::Request(_)
            | ProviderError::UnsupportedOption(_) => Self::Request(Box::new(err)),
            // `ProviderError` is `#[non_exhaustive]`, so a `rig` upgrade can add
            // a variant the compiler won't flag here. The warn is the cue to
            // classify it above; the error itself is not logged, as it may
            // carry the reply.
            _ => {
                tracing::warn!(target: TARGET, "unmapped rig ProviderError variant");
                Self::Response(ResponseFault::Unclassified)
            }
        }
    }
}

/// Which [`Error`] a relayed failure becomes, by the kind its report names.
enum Relayed {
    Transport,
    Provider,
    Request,
    /// A reply failure, kept as only its code: the report's message is the
    /// original's text.
    Reply(Option<String>),
}

impl Relayed {
    /// The class of the failure `report` relayed, by its original kind, so a
    /// relayed transport failure stays a (retryable) transport failure.
    fn of(report: &ErrorReport) -> Self {
        match report.kind {
            RigErrorKind::Http | RigErrorKind::Timeout => Self::Transport,
            RigErrorKind::Provider | RigErrorKind::ProviderResponse => Self::Provider,
            RigErrorKind::Url | RigErrorKind::Request => Self::Request,
            _ => Self::Reply(report.code.clone()),
        }
    }

    /// The error for a relayed failure of this class, carrying `err`.
    fn error(self, err: impl std::error::Error + Send + Sync + 'static) -> Error {
        match self {
            Self::Transport => Error::Transport(Box::new(err)),
            Self::Provider => Error::Provider(Box::new(err)),
            Self::Request => Error::Request(Box::new(err)),
            Self::Reply(code) => Error::Response(ResponseFault::Relayed { code }),
        }
    }
}

impl From<PromptError> for Error {
    fn from(err: PromptError) -> Self {
        match err {
            PromptError::Provider(e) => Self::from(e),
            PromptError::Report(ref report) => Relayed::of(report).error(err),
            // A stopped run carries its chat history, the document included;
            // only why it stopped is kept.
            PromptError::MaxTurns { max_turns, .. } => Self::Response(ResponseFault::Stopped(
                format!("reached the turn limit of {max_turns}"),
            )),
            PromptError::Cancelled { .. } => {
                Self::Response(ResponseFault::Stopped("cancelled".to_owned()))
            }
            // A memory failure, or a variant a `rig` upgrade added: the warn is
            // the cue to classify it above; the error itself is not logged.
            _ => {
                tracing::warn!(target: TARGET, "unmapped rig PromptError variant");
                Self::Response(ResponseFault::Unclassified)
            }
        }
    }
}

impl From<StructuredOutputError> for Error {
    fn from(err: StructuredOutputError) -> Self {
        match err {
            StructuredOutputError::Prompt(e) => Self::from(e),
            StructuredOutputError::Deserialization { error, .. } => {
                Self::Response(ResponseFault::Unparsable(JsonFault::from(&error)))
            }
            StructuredOutputError::EmptyResponse => Self::Response(ResponseFault::Empty),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A candidate batch, the shape a model's reply is parsed into.
    #[derive(Debug, serde::Deserialize)]
    #[allow(dead_code)]
    struct Batch {
        confidence: f64,
    }

    fn parse_failure(reply: &str) -> serde_json::Error {
        serde_json::from_str::<Batch>(reply).unwrap_err()
    }

    /// Neither the display nor the debug form of `err` mentions `secret`.
    fn assert_content_free(err: Error, secret: &str) {
        let core = CoreError::from(err);
        assert!(!core.to_string().contains(secret), "{core}");
        assert!(!format!("{core:?}").contains(secret), "{core:?}");
    }

    #[test]
    fn an_unparsable_reply_is_reported_without_it() {
        let reply = r#"{"confidence": "Jane Doe"}"#;
        let err = Error::from(StructuredOutputError::Deserialization {
            output: reply.to_owned(),
            error: parse_failure(reply),
        });
        let message = err.to_string();
        assert!(message.contains("a data error at line 1"), "{message}");
        assert_content_free(err, "Jane Doe");
    }

    #[test]
    fn a_json_provider_error_is_reported_without_the_reply() {
        let reply = r#"{"confidence": "Jane Doe"}"#;
        let err = Error::from(ProviderError::Json(parse_failure(reply).into()));
        assert_content_free(err, "Jane Doe");
    }

    #[test]
    fn a_malformed_reply_body_is_dropped() {
        let err = Error::from(ProviderError::Response(
            "Jane Doe lives in Berlin".to_owned(),
        ));
        assert_content_free(err, "Jane Doe");
    }

    #[test]
    fn a_stopped_run_is_reported_without_its_history() {
        let prompt = rig::completion::Message::user("Jane Doe lives in Berlin");
        let err = Error::from(PromptError::MaxTurns {
            max_turns: 3,
            chat_history: vec![prompt.clone()],
            prompt,
        });
        assert!(err.to_string().contains("turn limit of 3"));
        assert_content_free(err, "Jane Doe");
    }
}
