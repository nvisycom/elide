//! Calendar-date structural validator.

/// Return `true` if `value` is a real calendar date in one of the
/// supported written formats.
///
/// Supported formats are `MM/DD/YYYY`, `DD/MM/YYYY`, `YYYY-MM-DD`,
/// and `YYYY/MM/DD`, with `/` or `-` as separators. Leap years
/// are honoured and the year must fall in `1900..=2100`.
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

    let (year, month, day) = if parts[0].len() == 4 {
        // YYYY-MM-DD or YYYY/MM/DD
        match (parts[0].parse(), parts[1].parse(), parts[2].parse()) {
            (Ok(y), Ok(m), Ok(d)) => (y, m, d),
            _ => return false,
        }
    } else if parts[2].len() == 4 {
        // Could be MM/DD/YYYY or DD/MM/YYYY.
        let a: u32 = match parts[0].parse() {
            Ok(v) => v,
            Err(_) => return false,
        };
        let b: u32 = match parts[1].parse() {
            Ok(v) => v,
            Err(_) => return false,
        };
        let y: u32 = match parts[2].parse() {
            Ok(v) => v,
            Err(_) => return false,
        };

        // Try MM/DD/YYYY first, fall back to DD/MM/YYYY.
        if (1..=12).contains(&a) && is_valid_date(y, a, b) {
            (y, a, b)
        } else if (1..=12).contains(&b) && is_valid_date(y, b, a) {
            (y, b, a)
        } else {
            return false;
        }
    } else {
        return false;
    };

    is_valid_date(year, month, day)
}

fn is_valid_date(year: u32, month: u32, day: u32) -> bool {
    if !(1900..=2100).contains(&year) {
        return false;
    }
    if !(1..=12).contains(&month) {
        return false;
    }

    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => return false,
    };

    (1..=max_day).contains(&day)
}

fn is_leap_year(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
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
        // 13/01/1990 is valid as DD/MM/YYYY (Jan 13)
        assert!(date("13/01/1990"));
        // YYYY-MM-DD format: month 13 is invalid
        assert!(!date("1990-13-01"));
    }

    #[test]
    fn invalid_year() {
        assert!(!date("01/01/1899"));
        assert!(!date("1899-01-01"));
    }

    #[test]
    fn dd_mm_yyyy_ambiguous() {
        // 15/01/1990: first part > 12 so must be DD/MM
        assert!(date("15/01/1990"));
    }
}
