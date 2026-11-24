//! The [`ElideError`] every fallible export throws, as a real JS class.
//!
//! A `wasm_bindgen` class rather than a bare `Error`, so a caller can tell an
//! elide pipeline failure from any other with `e instanceof ElideError` and
//! branch on its [`kind`](ElideError::kind).

use elide::primitive::LanguageTag;
use serde::de::DeserializeOwned;
use tsify::{Ts, Tsify};
use wasm_bindgen::prelude::*;

/// The category of an [`ElideError`], mirroring the toolkit's own error kinds.
///
/// `wasm_bindgen` exports this as a TypeScript enum, so a caller can match on a
/// stable variant rather than parse a message.
#[wasm_bindgen]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElideErrorKind {
    /// The input was not valid for the format it was decoded as.
    MalformedInput,
    /// A pipeline component was misconfigured.
    Configuration,
    /// A detect / redact step failed while running.
    Processing,
    /// A resource limit (size, depth, count) was exceeded.
    ResourceLimit,
    /// A requested capability (modality, codec) is not available.
    CapabilityUnavailable,
    /// Recognition failed.
    Recognition,
    /// Redaction failed.
    Redaction,
    /// An encoded document failed its hidden-content check.
    Sanitization,
    /// An integrity or consistency check failed.
    Integrity,
    /// Something addressed is not there (a report part).
    NotFound,
    /// A backing provider (a JS callback backend) failed.
    Provider,
    /// A transport / network step failed.
    Transport,
    /// An error kind this binding does not model explicitly.
    Other,
}

impl From<elide::ErrorKind> for ElideErrorKind {
    fn from(kind: elide::ErrorKind) -> Self {
        match kind {
            elide::ErrorKind::MalformedInput => Self::MalformedInput,
            elide::ErrorKind::Configuration => Self::Configuration,
            elide::ErrorKind::Processing => Self::Processing,
            elide::ErrorKind::ResourceLimit => Self::ResourceLimit,
            elide::ErrorKind::CapabilityUnavailable => Self::CapabilityUnavailable,
            elide::ErrorKind::Recognition => Self::Recognition,
            elide::ErrorKind::Redaction => Self::Redaction,
            elide::ErrorKind::Sanitization => Self::Sanitization,
            elide::ErrorKind::Integrity => Self::Integrity,
            elide::ErrorKind::NotFound => Self::NotFound,
            elide::ErrorKind::Provider => Self::Provider,
            elide::ErrorKind::Transport => Self::Transport,
            _ => Self::Other,
        }
    }
}

/// The error every fallible export throws, a JavaScript class with a kind.
///
/// A JavaScript `catch` receives an `ElideError` instance: `e instanceof
/// ElideError` distinguishes it from any other thrown value, and its
/// [`kind`](Self::kind) and [`message`](Self::message) describe the failure.
#[wasm_bindgen]
pub struct ElideError {
    kind: ElideErrorKind,
    message: String,
}

#[wasm_bindgen]
impl ElideError {
    /// The failure category.
    #[wasm_bindgen(getter)]
    pub fn kind(&self) -> ElideErrorKind {
        self.kind
    }

    /// A human-readable description of the failure.
    #[wasm_bindgen(getter)]
    pub fn message(&self) -> String {
        self.message.clone()
    }
}

impl ElideError {
    /// An error of `kind` with `message`, for a failure this crate raises itself
    /// (a bad config value, an unreadable callback result).
    pub(crate) fn new(kind: ElideErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// A [`Configuration`](ElideErrorKind::Configuration) error with `message`.
    pub(crate) fn configuration(message: impl Into<String>) -> Self {
        Self::new(ElideErrorKind::Configuration, message)
    }
}

/// The Rust value of the JS config `value`, or a configuration error naming
/// `what` it configures.
pub(crate) fn from_js<T>(value: &Ts<T>, what: &str) -> Result<T, ElideError>
where
    T: Tsify + DeserializeOwned,
    T::JsType: Clone,
{
    value
        .to_rust()
        .map_err(|e| ElideError::configuration(format!("invalid {what}: {e}")))
}

/// The Rust value of an optional JS config `value`, its default when absent.
pub(crate) fn from_js_or_default<T>(value: Option<&Ts<T>>, what: &str) -> Result<T, ElideError>
where
    T: Tsify + DeserializeOwned + Default,
    T::JsType: Clone,
{
    value.map_or_else(|| Ok(T::default()), |value| from_js(value, what))
}

impl From<elide::Error> for ElideError {
    fn from(error: elide::Error) -> Self {
        Self {
            kind: error.kind().into(),
            message: error.to_string(),
        }
    }
}

/// The BCP-47 `tag`, or a configuration error naming it.
pub(crate) fn language_tag(tag: &str) -> Result<LanguageTag, ElideError> {
    tag.parse()
        .map_err(|e| ElideError::configuration(format!("invalid language tag `{tag}`: {e}")))
}

/// The BCP-47 `tags`, or a configuration error naming the first invalid one.
pub(crate) fn language_tags(tags: &[String]) -> Result<Vec<LanguageTag>, ElideError> {
    tags.iter().map(|tag| language_tag(tag)).collect()
}
