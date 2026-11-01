//! The crate's error constructors.

use std::fmt::Display;

use elide_core::{Error, ErrorKind};

/// A [`MalformedInput`](ErrorKind::MalformedInput) error saying `message`.
pub(crate) fn malformed(message: impl Display) -> Error {
    Error::new(ErrorKind::MalformedInput, message.to_string())
}
