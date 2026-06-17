//! US ZIP / ZIP+4 sanity validator.

/// Return `true` if `value` is a plausible US ZIP code.
///
/// Accepts the 5-digit and 5-4 (`12345-1234`) forms; rejects the
/// reserved all-zeros prefix (`00000`) which is not assigned by the
/// USPS but is a frequent stand-in for "unknown".
pub fn postal_code(value: &str) -> bool {
    let digits: Vec<char> = value.chars().filter(char::is_ascii_digit).collect();
    if digits.len() != 5 && digits.len() != 9 {
        return false;
    }
    !digits[..5].iter().all(|c| *c == '0')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid() {
        assert!(postal_code("90210"));
        assert!(postal_code("97477-1234"));
        // USPS lowest assigned prefix is 00501 (Holtsville, NY).
        assert!(postal_code("00501"));
    }

    #[test]
    fn rejects_all_zero_prefix() {
        assert!(!postal_code("00000"));
        assert!(!postal_code("00000-1234"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!postal_code("1234"));
        assert!(!postal_code("123456"));
        assert!(!postal_code(""));
    }
}
