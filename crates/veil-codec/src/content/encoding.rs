//! Character encoding for text-based loaders.

use veil_core::{Error, ErrorKind, Result};

/// Character encoding used to decode raw bytes before parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextEncoding {
    /// UTF-8 (the default and by far the most common encoding).
    #[default]
    Utf8,
}

impl TextEncoding {
    /// Decode raw bytes to a UTF-8 string.
    ///
    /// # Errors
    ///
    /// Returns a validation error when the bytes are not valid for this
    /// encoding.
    pub fn decode_bytes(self, bytes: &[u8]) -> Result<String> {
        match self {
            Self::Utf8 => String::from_utf8(bytes.to_vec())
                .map_err(|e| Error::new(ErrorKind::Validation, format!("invalid UTF-8: {e}"))),
        }
    }
}
