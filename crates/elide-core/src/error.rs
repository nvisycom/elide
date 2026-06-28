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
}

impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Self {
        Self { kind, source: None }
    }
}

impl From<derive_builder::UninitializedFieldError> for Error {
    /// Bridge `derive_builder`'s missing-required-field error into a
    /// [`ErrorKind::Validation`] failure, so generated builders that declare
    /// `build_fn(error = "Error")` fail with the crate-wide error type.
    fn from(err: derive_builder::UninitializedFieldError) -> Self {
        Self::new(ErrorKind::Validation, err)
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
/// Deliberately small and `#[non_exhaustive]`: `elide-core` defines types
/// and traits, so most failures here are validation errors and the
/// fusion failures `elide` may surface. Recognizer and operator
/// implementations in downstream crates carry their own richer context
/// and convert into [`Error`] at the trait boundary, tagging it with the
/// matching kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// A value was outside its permitted range (e.g. a confidence outside
    /// `0.0..=1.0`).
    OutOfRange,
    /// A merge was attempted over an empty set of detections.
    EmptyMerge,
    /// A group of detections could not be reconciled into one entity.
    Merge,
    /// A recognizer failed while inspecting content.
    Recognition,
    /// An operator failed while transforming content.
    Redaction,
    /// A configuration or rule was malformed: a bad regex, an unknown
    /// validator, a builder missing a required field.
    Validation,
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
            Self::OutOfRange => "value out of range",
            Self::EmptyMerge => "cannot merge an empty set of detections",
            Self::Merge => "detections could not be merged",
            Self::Recognition => "recognition failed",
            Self::Redaction => "redaction failed",
            Self::Validation => "validation failed",
            Self::Provider => "provider returned an error",
            Self::Transport => "transport failure",
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
