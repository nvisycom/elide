//! [`RedactRange`]: splice a redaction value into a byte range of a string.

use std::ops::Range;

use elide_core::{Error, ErrorKind, Result};

/// Apply a redaction to a string in place: replace a byte range with a
/// replacement value. The text-shaped handlers use it to splice a redacted
/// span back into a decoded value.
pub trait RedactRange {
    /// Replace `self[range]` with `value` in place.
    ///
    /// The range endpoints are clamped to the string length; an empty range is
    /// a no-op.
    ///
    /// # Errors
    ///
    /// Returns a redaction error if either endpoint falls mid-character.
    fn redact_range(&mut self, value: &str, range: Range<usize>) -> Result<()>;
}

impl RedactRange for String {
    fn redact_range(&mut self, value: &str, range: Range<usize>) -> Result<()> {
        let s = range.start.min(self.len());
        let e = range.end.min(self.len());
        if s >= e {
            return Ok(());
        }
        if !self.is_char_boundary(s) || !self.is_char_boundary(e) {
            return Err(Error::new(
                ErrorKind::Redaction,
                format!(
                    "redaction offset falls mid-character (start={}, end={}, len={})",
                    range.start,
                    range.end,
                    self.len()
                ),
            ));
        }
        self.replace_range(s..e, value);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
    fn out_of_bounds_clipped() {
        let mut s = String::from("short");
        s.redact_range("[X]", 0..999).unwrap();
        assert_eq!(s, "[X]");
    }

    #[test]
    fn mid_character_rejected() {
        let mut s = String::from("héllo"); // 'é' is 2 bytes
        let err = s.redact_range("[X]", 0..2).unwrap_err();
        assert!(err.to_string().contains("mid-character"));
    }
}
