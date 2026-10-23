//! Calendar-date structural validator.
//!
//! Calendar correctness (month lengths, leap years) is delegated to
//! [`jiff::civil::Date`]; this module owns only the written-format parsing and
//! the `MM/DD` vs `DD/MM` disambiguation, which `jiff` does not infer.

use jiff::civil::Date;

/// Return `true` if `value` is a real calendar date in one of the
/// supported written formats.
///
/// Supported formats are `MM/DD/YYYY`, `DD/MM/YYYY`, `YYYY-MM-DD`,
/// and `YYYY/MM/DD`, with `/` or `-` as separators. Leap years and
/// month lengths are validated by [`jiff::civil::Date`], and the year
/// must fall in `1900..=2100`.
///
/// # Ambiguity
///
/// When both interpretations are valid (e.g. `02/03/1999` could
/// mean Feb 3 or 3 Mar), the validator prefers `MM/DD/YYYY` and
/// only falls back to `DD/MM/YYYY` when the first part is not a
/// valid month. Locale disambiguation is out of scope.
pub fn date(value: &str) -> bool {
    let parts: Vec<&str> = value.split(['/', '-']).collect();
    if parts.len() != 3 {
        return false;
    }

    if parts[0].len() == 4 {
        // YYYY-MM-DD or YYYY/MM/DD.
        let (Ok(y), Ok(m), Ok(d)) = (parts[0].parse(), parts[1].parse(), parts[2].parse()) else {
            return false;
        };
        is_valid_date(y, m, d)
    } else if parts[2].len() == 4 {
        // MM/DD/YYYY or DD/MM/YYYY: prefer MM/DD, fall back to DD/MM.
        let (Ok(a), Ok(b), Ok(y)) = (parts[0].parse(), parts[1].parse(), parts[2].parse()) else {
            return false;
        };
        is_valid_date(y, a, b) || is_valid_date(y, b, a)
    } else {
        false
    }
}

/// A real calendar date in `year`/`month`/`day`, within the accepted year range.
fn is_valid_date(year: i16, month: i8, day: i8) -> bool {
    (1900..=2100).contains(&year) && Date::new(year, month, day).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mm_dd_yyyy() {
        assert!(date("01/15/1990"));
        assert!(date("12-31-2000"));
    }

    #[test]
    fn yyyy_mm_dd() {
        assert!(date("1990-01-15"));
        assert!(date("2000/12/31"));
    }

    #[test]
    fn leap_year() {
        assert!(date("02/29/2000"));
        assert!(date("2000-02-29"));
        assert!(!date("02/29/2001"));
    }

    #[test]
    fn invalid_day() {
        assert!(!date("04/31/1990"));
        assert!(!date("01/32/1990"));
        assert!(!date("01/00/1990"));
    }

    #[test]
    fn invalid_month() {
        // 13/01/1990 is valid as DD/MM/YYYY (Jan 13).
        assert!(date("13/01/1990"));
        // YYYY-MM-DD format: month 13 is invalid.
        assert!(!date("1990-13-01"));
    }

    #[test]
    fn invalid_year() {
        assert!(!date("01/01/1899"));
        assert!(!date("1899-01-01"));
    }

    #[test]
    fn dd_mm_yyyy_ambiguous() {
        // 15/01/1990: first part > 12 so must be DD/MM.
        assert!(date("15/01/1990"));
    }

    #[test]
    fn rejects_non_numeric_and_overflow() {
        assert!(!date("ab/cd/1990"));
        // Out of i8 range — parse fails rather than panicking.
        assert!(!date("999/01/1990"));
        assert!(!date("01/999/1990"));
    }
}
