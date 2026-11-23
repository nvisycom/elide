//! Byte-range reads into decoded text, shared by the handlers.

use std::ops::Range;

#[cfg(any(feature = "txt", feature = "json"))]
use elide_core::modality::text::TextLocation;
use elide_core::{Error, Result};

/// The span `location` addresses in a text's decoded stream.
///
/// # Errors
///
/// [`MalformedInput`] for an unresolved mark, which has none.
///
/// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
#[cfg(any(feature = "txt", feature = "json"))]
pub(crate) fn span(location: &TextLocation) -> Result<&Range<usize>> {
    location
        .range()
        .ok_or_else(|| Error::malformed("an unresolved mark has no span"))
}

/// The bytes of `text` at `range`.
///
/// # Errors
///
/// [`MalformedInput`] if the range is reversed, runs past the end, or splits
/// a UTF-8 character.
///
/// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
pub(crate) fn slice<'a>(text: &'a str, range: &Range<usize>) -> Result<&'a str> {
    text.get(range.clone()).ok_or_else(|| {
        Error::malformed(format!(
            "range {}..{} is outside the text ({} bytes) or splits a character",
            range.start,
            range.end,
            text.len()
        ))
    })
}
