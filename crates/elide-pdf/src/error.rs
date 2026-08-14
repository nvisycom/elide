//! [`Error`], [`ErrorKind`], and [`Result`]: coded, matchable failures.

use thiserror::Error as ThisError;

/// Convenience alias for this crate's results.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// A PDF extraction or rewrite failure, carrying a matchable
/// [`kind`](Error::kind) and a human-readable message.
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

    /// An [`ErrorKind::InvalidDocument`] error.
    pub(crate) fn invalid_document(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidDocument, message)
    }

    /// An [`ErrorKind::UnsafeRewrite`] error: a redaction is refused rather than
    /// applied unsafely. Used by every redaction path (glyph deletion, raster,
    /// image write-back).
    pub(crate) fn unsafe_rewrite(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::UnsafeRewrite, message)
    }

    /// An [`ErrorKind::LimitExceeded`] error.
    pub(crate) fn limit_exceeded(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::LimitExceeded, message)
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
    /// The bytes are not a readable PDF document.
    InvalidDocument,
    /// Parsing exceeded a configured bound (decompressed size), refused to
    /// protect against a decompression bomb.
    LimitExceeded,
    /// A redaction could not be applied safely and was refused rather than
    /// emitting a partially-redacted document — for example a page draws text
    /// with a font whose encoding cannot be decoded (so its glyphs cannot be
    /// located for deletion), an image replacement names a non-image object, or
    /// a raster page's pixels do not match its dimensions.
    UnsafeRewrite,
}
