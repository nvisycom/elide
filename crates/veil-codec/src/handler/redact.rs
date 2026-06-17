//! Shared byte-level string redaction helper used by the text-family
//! handlers.
//!
//! Replaces `buf[range]` with `value`. Clamps the range endpoints to
//! `buf.len()`; errors when either endpoint falls mid-character.

use std::ops::Range;

use veil_core::{Error, ErrorKind};

/// Replace `buf[range]` with `value` in place.
///
/// The range endpoints are clamped to `buf.len()`; an empty range is a
/// no-op.
///
/// # Errors
///
/// Returns a validation error if either endpoint falls mid-character.
pub(crate) fn replace_range(buf: &mut String, value: &str, range: Range<usize>) -> Result<(), Error> {
    let s = range.start.min(buf.len());
    let e = range.end.min(buf.len());
    if s >= e {
        return Ok(());
    }
    if !buf.is_char_boundary(s) || !buf.is_char_boundary(e) {
        return Err(Error::new(
            ErrorKind::Validation,
            format!(
                "redaction offset falls mid-character (start={}, end={}, len={})",
                range.start,
                range.end,
                buf.len()
            ),
        ));
    }
    buf.replace_range(s..e, value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_replacement() {
        let mut s = String::from("hello world");
        replace_range(&mut s, "[X]", 0..5).unwrap();
        assert_eq!(s, "[X] world");
    }

    #[test]
    fn remove_empty_value() {
        let mut s = String::from("hello world");
        replace_range(&mut s, "", 5..11).unwrap();
        assert_eq!(s, "hello");
    }

    #[test]
    fn out_of_bounds_clipped() {
        let mut s = String::from("short");
        replace_range(&mut s, "[X]", 0..999).unwrap();
        assert_eq!(s, "[X]");
    }

    #[test]
    fn mid_character_rejected() {
        let mut s = String::from("héllo"); // 'é' is 2 bytes
        let err = replace_range(&mut s, "[X]", 0..2).unwrap_err();
        assert!(err.to_string().contains("mid-character"));
    }
}
