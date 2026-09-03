//! [`Original`]: the source value a fake replaces, with helpers for the
//! generators that need to read it.

use std::str::FromStr;

/// The original value being faked, threaded through the catalogue.
///
/// A thin wrapper over the source string so a generator can inspect it, a bare
/// `&str` re-parsed at each call is easy to get subtly wrong. Free-form
/// generators ignore it; structured ones read its shape ([`as_str`](Self::as_str)),
/// and some (like age) parse a semantic value from it ([`number`](Self::number)).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Original<'a>(&'a str);

impl<'a> Original<'a> {
    /// Wrap a source string.
    pub(crate) fn new(value: &'a str) -> Self {
        Self(value)
    }

    /// The raw source string.
    pub(crate) fn as_str(&self) -> &'a str {
        self.0
    }

    /// Whether the source is empty, no pattern to preserve, no value to parse.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }

    /// Parse the (trimmed) source as a number of type `T`, `None` when it isn't
    /// one.
    pub(crate) fn number<T: FromStr>(&self) -> Option<T> {
        self.0.trim().parse::<T>().ok()
    }

    /// The count of significant digits in the whole-number part, the source's
    /// order of magnitude, so a generator can preserve it. `None` when the
    /// source has no digits.
    ///
    /// A trailing fractional group (the digits after the *last* `.` or `,`, when
    /// that group is one or two digits, the shape of a decimal fraction) is
    /// excluded, so `"$2,000,000.00"` and `"1.234,56 €"` both report `7`.
    /// Leading zeros are counted, since an id's width is meaningful even when
    /// zero-padded.
    pub(crate) fn digit_magnitude(&self) -> Option<u32> {
        let whole = strip_fraction(self.0);
        let count = whole.chars().filter(char::is_ascii_digit).count() as u32;
        (count > 0).then_some(count)
    }

    /// Whether the whole-number part is all zeros (`"0"`, `"0.50"`, `"$0.00"`),
    /// so a magnitude-preserving generator can emit a zero whole part rather than
    /// inflating it to a nonzero digit.
    pub(crate) fn whole_is_zero(&self) -> bool {
        all_zero(strip_fraction(self.0))
    }

    /// Whether the *entire* numeric value is zero (`"0"`, `"0.00"`, `"$0.00"`),
    /// both the whole part and any fraction, so a generator can keep it exactly
    /// zero rather than a nonzero sub-unit value.
    pub(crate) fn is_zero(&self) -> bool {
        all_zero(self.0)
    }
}

/// Whether `s` has at least one digit and all its digits are `0`.
fn all_zero(s: &str) -> bool {
    let mut digits = s.chars().filter(char::is_ascii_digit).peekable();
    digits.peek().is_some() && digits.all(|c| c == '0')
}

/// Drop a trailing fractional group: the run of digits after the final `.` or
/// `,`, when it is one or two digits long (a decimal fraction, not a thousands
/// group). Everything else is left as the whole part.
///
/// Trailing non-digit clutter (a currency symbol, a space) is ignored first, so
/// `"1.234.567,89 €"` sees `89` as the fraction, not `89 €`.
fn strip_fraction(s: &str) -> &str {
    let s = s.trim_end_matches(|c: char| !c.is_ascii_digit());
    match s.rfind(['.', ',']) {
        Some(i) => {
            let frac = &s[i + 1..];
            let is_fraction =
                matches!(frac.len(), 1 | 2) && frac.chars().all(|c| c.is_ascii_digit());
            if is_fraction { &s[..i] } else { s }
        }
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_parses_trimmed() {
        assert_eq!(Original::new("  42 ").number::<u8>(), Some(42));
        assert_eq!(Original::new("ninety").number::<u8>(), None);
    }

    #[test]
    fn digit_magnitude_counts_whole_digits_excluding_a_fraction() {
        // Millions, either separator style, fraction excluded.
        assert_eq!(Original::new("$2,000,000.00").digit_magnitude(), Some(7));
        assert_eq!(Original::new("1.234.567,89 €").digit_magnitude(), Some(7));
        // European style, thousands: 1234 whole (the ",56" is the fraction).
        assert_eq!(Original::new("1.234,56 €").digit_magnitude(), Some(4));
        // No fraction: every digit counts.
        assert_eq!(Original::new("4471").digit_magnitude(), Some(4));
        assert_eq!(Original::new("00219938").digit_magnitude(), Some(8));
        // A thousands group is not a fraction (3 digits after the mark).
        assert_eq!(Original::new("1,000").digit_magnitude(), Some(4));
        // No digits at all.
        assert_eq!(Original::new("n/a").digit_magnitude(), None);
    }

    #[test]
    fn whole_is_zero_detects_sub_unit_and_zero_amounts() {
        assert!(Original::new("0").whole_is_zero());
        assert!(Original::new("0.50").whole_is_zero());
        assert!(Original::new("$0.00").whole_is_zero());
        assert!(!Original::new("42.00").whole_is_zero());
        assert!(!Original::new("100").whole_is_zero());
        // No digits: not a zero whole part.
        assert!(!Original::new("n/a").whole_is_zero());
    }

    #[test]
    fn is_zero_requires_the_whole_value_to_be_zero() {
        // Entirely zero.
        assert!(Original::new("0").is_zero());
        assert!(Original::new("0.00").is_zero());
        assert!(Original::new("$0.00").is_zero());
        // A nonzero fraction is not zero, even with a zero whole part.
        assert!(!Original::new("0.50").is_zero());
        assert!(!Original::new("100").is_zero());
        assert!(!Original::new("n/a").is_zero());
    }
}
