//! US National Provider Identifier (NPI) checksum validator.
//!
//! See `assets/NOTICE.md` for third-party attribution.

use super::super::luhn::luhn;

/// Return `true` if `value` is a valid 10-digit US NPI.
///
/// The CMS algorithm prepends the constant `"80840"` to the
/// 10-digit identifier and runs the standard Luhn checksum on
/// the resulting 15-digit string.
///
/// Whitespace and `-` separators are stripped before validation.
pub fn npi(value: &str) -> bool {
    let digits: String = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-')
        .collect();
    if digits.len() != 10 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    // Reject all-same-digit bodies (e.g. `1111111111`); they pass
    // Luhn but are not real provider numbers.
    let body = &digits[..9];
    if body.chars().all(|c| c == body.chars().next().unwrap()) {
        return false;
    }
    let prefixed = format!("80840{digits}");
    luhn(&prefixed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_known_npi_number() {
        // Test vector validated against the CMS Luhn-on-80840 algorithm.
        assert!(npi("1234567893"));
    }

    #[test]
    fn strips_separators() {
        assert!(npi("1234-567-893"));
        assert!(npi("1234 567 893"));
    }

    #[test]
    fn rejects_wrong_check_digit() {
        assert!(!npi("1234567890"));
        assert!(!npi("1234567899"));
    }

    #[test]
    fn rejects_degenerate_all_same_digits() {
        assert!(!npi("1111111111"));
        assert!(!npi("2222222222"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!npi("123456789"));
        assert!(!npi("12345678901"));
        assert!(!npi(""));
    }
}
