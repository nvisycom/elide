//! [`Error`], [`ErrorKind`], and [`Result`]: coded, matchable failures.

use thiserror::Error as ThisError;

/// Convenience alias for this crate's results.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// A DOCX extraction or rewrite failure, carrying a matchable
/// [`kind`](Error::kind) and a human-readable message.
///
/// The kind is stable and coarse so a caller can branch (retry, reject,
/// report "unsupported document") without string-matching the message.
#[derive(Debug, ThisError)]
#[error("{kind:?}: {message}")]
pub struct Error {
    kind: ErrorKind,
    message: String,
}

impl Error {
    fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// An [`ErrorKind::InvalidArchive`] error.
    pub(crate) fn invalid_archive(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidArchive, message)
    }

    /// An [`ErrorKind::InvalidPackage`] error.
    pub(crate) fn invalid_package(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidPackage, message)
    }

    /// An [`ErrorKind::InvalidXml`] error.
    pub(crate) fn invalid_xml(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidXml, message)
    }

    /// An [`ErrorKind::UnsafeRewrite`] error.
    pub(crate) fn unsafe_rewrite(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::UnsafeRewrite, message)
    }

    /// The matchable failure category.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
}

/// Coarse category of an [`Error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The bytes are not a readable zip archive.
    InvalidArchive,
    /// The archive is a zip but not a well-formed DOCX package (missing the
    /// body part, or its required structure).
    InvalidPackage,
    /// A part that should be XML failed to parse, or its text is not UTF-8.
    InvalidXml,
    /// A rewrite could not be applied safely (a replacement span is out of
    /// bounds, overlaps another, or falls mid-character). The rewrite is
    /// refused rather than emitting a partially-redacted document.
    UnsafeRewrite,
}
