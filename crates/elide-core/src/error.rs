//! Crate-wide error type.
//!
//! Modelled on [`std::io::Error`]: a single opaque [`Error`] struct
//! pairs a coarse, matchable [`ErrorKind`] with an optional boxed cause.
//! Callers match on the kind for control flow while the underlying
//! source (a recognizer's failure, an operator's failure) travels
//! along for diagnostics without widening the public enum. New failure
//! modes can be added as kinds without breaking the struct's API.

use std::fmt;

/// Type-erased, thread-safe error cause.
///
/// The boxed form a downstream recognizer or operator error is stored in
/// when attached to an [`Error`] as its underlying source.
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Error type returned across the core domain operations.
///
/// Opaque by design: construct one with [`Error::new`] (kind + cause) or
/// [`Error::from`] (kind only), inspect it with [`Error::kind`], and
/// recover the cause, if any, with [`Error::into_source`] or the standard
/// [`source`].
///
/// [`source`]: std::error::Error::source
pub struct Error {
    kind: ErrorKind,
    source: Option<BoxError>,
}

impl Error {
    /// Build an error of `kind` wrapping an underlying `source` cause.
    pub fn new<E>(kind: ErrorKind, source: E) -> Self
    where
        E: Into<BoxError>,
    {
        Self {
            kind,
            source: Some(source.into()),
        }
    }

    /// Coarse category of this error, for control-flow matching.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Take the underlying cause, if one was attached.
    pub fn into_source(self) -> Option<BoxError> {
        self.source
    }

    /// Whether retrying the failed operation unchanged could plausibly
    /// succeed. A property of this error's [`kind`](Self::kind).
    ///
    /// Only transport-layer failures ([`ErrorKind::Transport`]) are treated as
    /// retryable: a network hiccup or timeout may clear on a second attempt.
    /// Every other kind is deterministic (a malformed document, a bad rule, a
    /// missing capability, or an external service that answered with an error
    /// via [`ErrorKind::Provider`]) and will fail the same way again, so a
    /// caller should surface it rather than loop.
    ///
    /// See [`ErrorKind::is_retryable`] for the per-kind classification.
    pub fn is_retryable(&self) -> bool {
        self.kind.is_retryable()
    }
}

impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Self {
        Self { kind, source: None }
    }
}

impl From<derive_builder::UninitializedFieldError> for Error {
    /// Bridge `derive_builder`'s missing-required-field error into a
    /// [`ErrorKind::Configuration`] failure, so generated builders that
    /// declare `build_fn(error = "Error")` fail with the crate-wide error
    /// type.
    fn from(err: derive_builder::UninitializedFieldError) -> Self {
        Self::new(ErrorKind::Configuration, err)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.source {
            Some(source) => write!(f, "{}: {source}", self.kind),
            None => fmt::Display::fmt(&self.kind, f),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Error")
            .field("kind", &self.kind)
            .field("source", &self.source)
            .finish()
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|s| s.as_ref() as &(dyn std::error::Error + 'static))
    }
}

/// Coarse category of [`Error`], suitable for matching.
///
/// Deliberately small and `#[non_exhaustive]`: it names the *kind* of
/// failure a caller can branch on — is the input corrupt, the config
/// wrong, a capability missing, an external service at fault — while the
/// boxed cause carries the detail. Downstream crates convert their richer
/// errors into an [`Error`] at the trait boundary, tagging it with the
/// kind that fits.
///
/// The distinctions worth branching on:
///
/// - [`MalformedInput`] vs [`Configuration`]: bad *content* the caller fed
///   in (a corrupt file) versus a bad *rule* the caller wrote (an invalid
///   regex). One means "fix the document", the other "fix your setup".
/// - [`CapabilityUnavailable`]: the request is well-formed but the
///   component that would serve it is absent — match this to fall back or
///   report "unsupported" rather than "malformed".
/// - [`Provider`] vs [`Transport`]: an external service answered with an
///   error versus was never reached — the first is not retryable, the
///   second often is.
///
/// [`MalformedInput`]: Self::MalformedInput
/// [`Configuration`]: Self::Configuration
/// [`CapabilityUnavailable`]: Self::CapabilityUnavailable
/// [`Provider`]: Self::Provider
/// [`Transport`]: Self::Transport
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The content being processed is corrupt or unreadable: a malformed
    /// document, invalid UTF-8, an audio stream missing its track params.
    /// The caller's *input* is at fault, not their configuration.
    MalformedInput,
    /// A caller-supplied rule or configuration is invalid: a bad regex, a
    /// malformed rule file (TOML/CSV), a prompt template that won't parse,
    /// a builder missing a required field. The caller's *setup* is at
    /// fault, not the input it was run over.
    Configuration,
    /// A valid operation failed while running: an encoder rejected its
    /// input, an encrypt/decrypt step failed. Distinct from
    /// [`MalformedInput`]: the inputs were well-formed, the operation
    /// itself did not complete.
    ///
    /// [`MalformedInput`]: Self::MalformedInput
    Processing,
    /// A requested capability is not wired up: no codec registered for a
    /// format, no backend for a modality, no handler for a container part.
    /// Distinct from [`Configuration`]: the request is well-formed, but the
    /// component that would serve it is absent (a feature left unbuilt, a
    /// slot never configured), so a caller can match this to fall back or
    /// report "unsupported" rather than "misconfigured".
    ///
    /// [`Configuration`]: Self::Configuration
    CapabilityUnavailable,
    /// A recognizer failed while inspecting content.
    Recognition,
    /// A redaction operator failed while transforming content.
    Redaction,
    /// A tamper-evident structure failed verification: an entity's audit
    /// trail (an [`AuditLog`]) no longer matches its recorded hashes, so it
    /// was edited, reordered, or truncated after the fact. Distinct from
    /// [`MalformedInput`]: the structure is well-formed, but its integrity
    /// guarantee is broken.
    ///
    /// [`AuditLog`]: crate::entity::audit::AuditLog
    /// [`MalformedInput`]: Self::MalformedInput
    Integrity,
    /// An external provider (an LLM service, a hosted model) returned an
    /// error response.
    Provider,
    /// A transport-layer failure reaching an external service (HTTP,
    /// network, timeout).
    Transport,
}

impl ErrorKind {
    /// Stable, human-readable description of the kind.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MalformedInput => "input is malformed",
            Self::Configuration => "configuration is invalid",
            Self::Processing => "processing failed",
            Self::CapabilityUnavailable => "required capability is unavailable",
            Self::Recognition => "recognition failed",
            Self::Redaction => "redaction failed",
            Self::Integrity => "integrity verification failed",
            Self::Provider => "provider returned an error",
            Self::Transport => "transport failure",
        }
    }

    /// Whether an operation that failed with this kind could plausibly
    /// succeed if retried unchanged.
    ///
    /// Only [`Transport`](Self::Transport) is retryable: a network failure or
    /// timeout may be transient. Every other kind is deterministic: the same
    /// input, rule, or absent capability fails identically on a retry, and a
    /// [`Provider`](Self::Provider) that answered with an error is reporting a
    /// decision, not a transient fault. The `match` is exhaustive so a new
    /// kind must state its retryability rather than default silently.
    pub const fn is_retryable(self) -> bool {
        match self {
            Self::Transport => true,
            Self::MalformedInput
            | Self::Configuration
            | Self::Processing
            | Self::CapabilityUnavailable
            | Self::Recognition
            | Self::Redaction
            | Self::Integrity
            | Self::Provider => false,
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Convenience alias for results in this crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_transport_is_retryable() {
        assert!(ErrorKind::Transport.is_retryable());
        for kind in [
            ErrorKind::MalformedInput,
            ErrorKind::Configuration,
            ErrorKind::Processing,
            ErrorKind::CapabilityUnavailable,
            ErrorKind::Recognition,
            ErrorKind::Redaction,
            ErrorKind::Integrity,
            ErrorKind::Provider,
        ] {
            assert!(!kind.is_retryable(), "{kind} should not be retryable");
        }
    }

    #[test]
    fn error_retryability_follows_its_kind() {
        assert!(Error::from(ErrorKind::Transport).is_retryable());
        assert!(!Error::from(ErrorKind::Provider).is_retryable());
    }
}
