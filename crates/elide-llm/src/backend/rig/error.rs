//! The bridge from `rig`'s errors to [`elide_core::Error`].
//!
//! The crate exposes no error type of its own: every public result is an
//! [`elide_core::Error`]. This private [`Error`] hosts the `From<rig error>`
//! conversions the orphan rule forbids writing onto `elide_core::Error`
//! directly, and keeps call sites `?`-ergonomic. Its variants name the
//! [`ErrorKind`] each maps to.
//!
//! No `rig` error is kept. A model's reply echoes the document it was asked
//! about, so it is the document's PII, and rig carries replies on its errors:
//! the raw body of a failed response, the chat history of a stopped run, the
//! output that failed to parse; `serde_json`'s messages quote the values they
//! fail on. Each error is rebuilt as a [`Fault`]: what happened, in this
//! module's words, plus the metadata a provider gives that names no content
//! (the HTTP status, the provider's machine code, its request id).

use std::borrow::Cow;
use std::fmt;

use elide_core::{Error as CoreError, ErrorKind};
use rig::ProviderResponseError;
use rig::completion::{PromptError, StructuredOutputError};
use rig::error::{ErrorKind as RigErrorKind, ErrorReport, ProviderError};
use rig::http_client::Error as HttpClientError;
use serde_json::Value;
use serde_json::error::Category;

const TARGET: &str = "elide_llm::backend::rig::error";

/// Internal LLM error, tagged by the [`ErrorKind`] it maps to.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    /// A transport-layer failure reaching the provider (HTTP, network).
    #[error("transport: {0}")]
    Transport(Fault),

    /// The provider answered with a failure.
    #[error("provider: {0}")]
    Provider(Fault),

    /// The provider's reply arrived but is not a usable answer.
    #[error("response: {0}")]
    Response(Fault),

    /// Request construction failed (a bad URL or unsupported option).
    #[error("request: {0}")]
    Request(Fault),
}

/// A failure described without the content it was about.
#[derive(Debug)]
pub(crate) struct Fault {
    /// What happened, in this module's words, or from a source whose message
    /// names no content (an HTTP client's connection failure, a URL parse).
    what: Cow<'static, str>,
    /// The HTTP status of the provider's reply.
    status: Option<u16>,
    /// The provider's machine-readable code (`rate_limit_exceeded`).
    code: Option<String>,
    /// The provider's id for the request, to look the failure up with it.
    request_id: Option<String>,
}

impl Fault {
    fn new(what: impl Into<Cow<'static, str>>) -> Self {
        Self {
            what: what.into(),
            status: None,
            code: None,
            request_id: None,
        }
    }

    fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    /// A provider's failed `response`: its status, code, and request id, not
    /// its body.
    fn response(what: &'static str, response: &ProviderResponseError) -> Self {
        Self {
            what: Cow::Borrowed(what),
            status: response.status.map(|status| status.as_u16()),
            code: response.code.clone().or_else(|| body_code(&response.body)),
            request_id: response.provider_request_id.clone(),
        }
    }

    /// A failure a relay reported: its status, code, and request id, not its
    /// message (the original error's text).
    fn report(report: &ErrorReport) -> Self {
        Self {
            what: Cow::Borrowed("a relayed failure"),
            status: report.http_status,
            code: report.code.clone(),
            request_id: report.request_id.clone(),
        }
    }

    /// A JSON parse failure: its category and position, not the message,
    /// which quotes the value it failed on.
    fn json(error: &serde_json::Error) -> Self {
        let category = match error.classify() {
            Category::Io => "an I/O error",
            Category::Syntax => "a syntax error",
            Category::Data => "a data error",
            Category::Eof => "an unexpected end of input",
        };
        Self::new(format!(
            "the reply did not parse: {category} at line {}, column {}",
            error.line(),
            error.column()
        ))
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.what)?;
        let mut details = Vec::new();
        if let Some(status) = self.status {
            details.push(format!("status {status}"));
        }
        if let Some(code) = &self.code {
            details.push(format!("code {code}"));
        }
        if let Some(request_id) = &self.request_id {
            details.push(format!("request {request_id}"));
        }
        if !details.is_empty() {
            write!(f, " ({})", details.join(", "))?;
        }
        Ok(())
    }
}

/// The provider's machine code in an error `body`: the `code`, `status`, or
/// `type` of its `error` object, the envelope OpenAI, Anthropic, and Gemini
/// share. Only a short identifier-like value is taken, never free text.
fn body_code(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    let error = value.get("error")?;
    ["code", "status", "type"].iter().find_map(|field| {
        let code = error.get(field)?.as_str()?;
        let is_code = !code.is_empty()
            && code.len() <= 64
            && code
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
        is_code.then(|| code.to_owned())
    })
}

/// The transport fault for an HTTP client failure. A failed status keeps the
/// status, not the body; the client's own connection and protocol messages
/// name no content and are kept.
fn transport(err: &HttpClientError) -> Fault {
    match err {
        HttpClientError::InvalidStatusCodeWithDetails { status, .. } => {
            Fault::new("the provider answered with a failure status").with_status(status.as_u16())
        }
        HttpClientError::Protocol(e) => Fault::new(format!("an HTTP protocol error: {e}")),
        HttpClientError::Instance(e) => Fault::new(format!("the HTTP client failed: {e}")),
        HttpClientError::InvalidHeaderValue(_) => Fault::new("an invalid header value"),
        HttpClientError::NoHeaders => Fault::new("the response had no headers"),
        HttpClientError::StreamEnded => Fault::new("the response stream ended"),
        HttpClientError::InvalidContentType(_) => {
            Fault::new("the response had an unexpected content type")
        }
    }
}

impl Error {
    /// A failure a relay reported, classified by its original kind, so a
    /// relayed transport failure stays a (retryable) transport failure.
    fn relayed(report: &ErrorReport) -> Self {
        let fault = Fault::report(report);
        match report.kind {
            RigErrorKind::Http | RigErrorKind::Timeout => Self::Transport(fault),
            RigErrorKind::Provider | RigErrorKind::ProviderResponse => Self::Provider(fault),
            RigErrorKind::Url | RigErrorKind::Request => Self::Request(fault),
            _ => Self::Response(fault),
        }
    }
}

impl From<ProviderError> for Error {
    fn from(err: ProviderError) -> Self {
        match err {
            // No reply reached us: a reset, timeout, or unreadable response.
            ProviderError::Http(e) => Self::Transport(transport(&e)),
            // The provider answered with a failure (an error status/envelope,
            // rejected credentials, an expired cache handle).
            ProviderError::Provider(_) => {
                Self::Provider(Fault::new("the provider reported a failure"))
            }
            ProviderError::ProviderResponse(response) => Self::Provider(Fault::response(
                "the provider answered with a failure",
                &response,
            )),
            ProviderError::InvalidAuthentication(response) => Self::Provider(Fault::response(
                "the provider rejected the credentials",
                &response,
            )),
            ProviderError::CacheExpired { response, .. } => Self::Provider(Fault::response(
                "the provider rejected the context-cache handle",
                &response,
            )),
            // A reply arrived but is not a usable answer to the request.
            ProviderError::Json(e) => Self::Response(Fault::json(&e)),
            ProviderError::Response(_) => {
                Self::Response(Fault::new("the provider's reply was malformed"))
            }
            ProviderError::Truncated => {
                Self::Response(Fault::new("the provider's reply was truncated"))
            }
            ProviderError::MismatchedDimensions { .. } => {
                Self::Response(Fault::new("the provider's reply had mismatched dimensions"))
            }
            ProviderError::Relayed(report) => Self::relayed(&report),
            // Caller-side: a request we could not build.
            ProviderError::Url(e) => Self::Request(Fault::new(format!("an invalid URL: {e}"))),
            ProviderError::Request(_) => {
                Self::Request(Fault::new("the request could not be built"))
            }
            ProviderError::UnsupportedOption(option) => Self::Request(Fault::new(format!(
                "the provider does not support the `{}` option",
                option.option
            ))),
            // `ProviderError` is `#[non_exhaustive]`, so a `rig` upgrade can add
            // a variant the compiler won't flag here. The warn is the cue to
            // classify it above.
            _ => {
                tracing::warn!(target: TARGET, "unmapped rig ProviderError variant");
                Self::Response(Fault::new("an unclassified provider failure"))
            }
        }
    }
}

impl From<PromptError> for Error {
    fn from(err: PromptError) -> Self {
        match err {
            PromptError::Provider(e) => Self::from(e),
            PromptError::Report(report) => Self::relayed(&report),
            PromptError::MaxTurns { max_turns, .. } => Self::Response(Fault::new(format!(
                "the extraction run stopped at its turn limit of {max_turns}"
            ))),
            PromptError::Cancelled { .. } => {
                Self::Response(Fault::new("the extraction run was cancelled"))
            }
            // A memory failure, or a variant a `rig` upgrade added: the warn is
            // the cue to classify it above.
            _ => {
                tracing::warn!(target: TARGET, "unmapped rig PromptError variant");
                Self::Response(Fault::new("an unclassified extraction failure"))
            }
        }
    }
}

impl From<StructuredOutputError> for Error {
    fn from(err: StructuredOutputError) -> Self {
        match err {
            StructuredOutputError::Prompt(e) => Self::from(e),
            StructuredOutputError::Deserialization { error, .. } => {
                Self::Response(Fault::json(&error))
            }
            StructuredOutputError::EmptyResponse => {
                Self::Response(Fault::new("the model returned no structured output"))
            }
        }
    }
}

impl From<HttpClientError> for Error {
    fn from(err: HttpClientError) -> Self {
        Self::Transport(transport(&err))
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
    use rig::http_client::{HeaderMap, StatusCode};

    use super::*;

    const SECRET: &str = "Jane Doe";

    /// A candidate batch, the shape a model's reply is parsed into.
    #[derive(Debug, serde::Deserialize)]
    #[allow(dead_code, reason = "parsed only to fail")]
    struct Batch {
        confidence: f64,
    }

    fn parse_failure() -> serde_json::Error {
        serde_json::from_str::<Batch>(r#"{"confidence": "Jane Doe"}"#).unwrap_err()
    }

    /// A provider's error reply that echoes the document, with a code.
    fn echoing_response() -> ProviderResponseError {
        ProviderResponseError::new(
            StatusCode::BAD_REQUEST,
            r#"{"error": {"code": "content_filter", "message": "flagged: Jane Doe"}}"#,
        )
    }

    /// Neither the display nor the debug form of `err` holds the secret.
    fn assert_content_free(err: impl Into<Error>) -> String {
        let core = CoreError::from(err.into());
        let (display, debug) = (core.to_string(), format!("{core:?}"));
        assert!(!display.contains(SECRET), "{display}");
        assert!(!debug.contains(SECRET), "{debug}");
        display
    }

    #[test]
    fn an_unparsable_reply_keeps_only_its_position() {
        let display = assert_content_free(StructuredOutputError::Deserialization {
            output: format!(r#"{{"confidence": "{SECRET}"}}"#),
            error: parse_failure(),
        });
        assert!(display.contains("a data error at line 1"), "{display}");
    }

    #[test]
    fn a_json_provider_error_keeps_only_its_position() {
        assert_content_free(ProviderError::Json(parse_failure().into()));
    }

    #[test]
    fn a_malformed_reply_body_is_dropped() {
        assert_content_free(ProviderError::Response(format!("{SECRET} lives in Berlin")));
    }

    #[test]
    fn a_provider_failure_keeps_its_status_and_code_not_its_body() {
        let display = assert_content_free(ProviderError::ProviderResponse(echoing_response()));
        assert!(display.contains("status 400"), "{display}");
        assert!(display.contains("code content_filter"), "{display}");
    }

    #[test]
    fn a_rejected_credential_keeps_its_status_not_its_body() {
        assert_content_free(ProviderError::InvalidAuthentication(echoing_response()));
    }

    #[test]
    fn a_failed_status_keeps_the_status_not_the_body() {
        let display = assert_content_free(HttpClientError::InvalidStatusCodeWithDetails {
            status: StatusCode::TOO_MANY_REQUESTS,
            body: format!("too many requests about {SECRET}"),
            headers: HeaderMap::new(),
        });
        assert!(display.contains("status 429"), "{display}");
    }

    #[test]
    fn a_relayed_failure_keeps_its_code_not_its_message() {
        let mut report = ErrorReport::new(RigErrorKind::ProviderResponse, SECRET);
        report.code = Some("overloaded".to_owned());
        report.provider_response = Some(echoing_response());
        let display = assert_content_free(ProviderError::Relayed(Box::new(report)));
        assert!(display.contains("code overloaded"), "{display}");
    }

    #[test]
    fn a_stopped_run_keeps_its_turn_limit_not_its_history() {
        let prompt = rig::completion::Message::user(format!("{SECRET} lives in Berlin"));
        let display = assert_content_free(PromptError::MaxTurns {
            max_turns: 3,
            chat_history: vec![prompt.clone()],
            prompt,
        });
        assert!(display.contains("turn limit of 3"), "{display}");
    }

    #[test]
    fn a_body_code_is_an_identifier_never_free_text() {
        assert_eq!(
            body_code(r#"{"error": {"code": "rate_limit_exceeded"}}"#).as_deref(),
            Some("rate_limit_exceeded")
        );
        assert_eq!(
            body_code(r#"{"error": {"type": "Jane Doe was flagged"}}"#),
            None
        );
        assert_eq!(body_code("not json"), None);
    }
}
