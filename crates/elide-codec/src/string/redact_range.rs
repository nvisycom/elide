//! [`RedactRange`]: splice a redaction value into a byte range of a string.

use std::ops::Range;

use elide_core::{Error, Result};

/// Splice a redaction into a string: replace a byte range with a value.
///
/// The text-shaped streams use it to splice a redacted span back into a decoded
/// value.
pub trait RedactRange {
    /// Replace `self[range]` with `value` in place. An empty range is a no-op.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] if the range is reversed, runs past the end, or
    /// splits a character: a redaction is never clamped or dropped.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    fn redact_range(&mut self, value: &str, range: Range<usize>) -> Result<()>;
}

impl RedactRange for String {
    fn redact_range(&mut self, value: &str, range: Range<usize>) -> Result<()> {
        if range.start > range.end || range.end > self.len() {
            return Err(Error::malformed(format!(
                "redaction range {}..{} is outside the text ({} bytes)",
                range.start,
                range.end,
                self.len()
            )));
        }
        if !self.is_char_boundary(range.start) || !self.is_char_boundary(range.end) {
            return Err(Error::malformed(format!(
                "redaction range {}..{} splits a UTF-8 character",
                range.start, range.end
            )));
        }
        if range.is_empty() {
            return Ok(());
        }
        self.replace_range(range, value);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;

    use super::*;

    #[test]
    fn single_replacement() {
        let mut s = String::from("hello world");
        s.redact_range("[X]", 0..5).unwrap();
        assert_eq!(s, "[X] world");
    }

    #[test]
    fn remove_empty_value() {
        let mut s = String::from("hello world");
        s.redact_range("", 5..11).unwrap();
        assert_eq!(s, "hello");
    }

    #[test]
    fn a_range_outside_the_text_is_an_error() {
        let mut s = String::from("short");
        let err = s.redact_range("[X]", 0..999).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        let err = s
            .redact_range("[X]", Range { start: 3, end: 1 })
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        assert_eq!(s, "short");
    }

    #[test]
    fn mid_character_rejected() {
        let mut s = String::from("héllo"); // 'é' is 2 bytes
        let err = s.redact_range("[X]", 0..2).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
    }

    #[test]
    fn an_empty_range_is_a_no_op() {
        let mut s = String::from("hello");
        s.redact_range("[X]", 2..2).unwrap();
        assert_eq!(s, "hello");
    }
}
