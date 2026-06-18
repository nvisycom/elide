//! UK NHS number checksum validator.
//!
//! See `assets/NOTICE.md` for third-party attribution.

/// Return `true` if `value` is a valid 10-digit UK NHS number.
///
/// The NHS algorithm multiplies each of the 10 digits by descending
/// weights `[10, 9, 8, …, 1]` and accepts the number when the sum
/// is divisible by 11. Equivalent to checking that the last digit
/// equals `(11 - (weighted_sum_of_first_9 % 11)) % 11`, rejecting
/// the special case where the expected check digit would be 10.
///
/// Whitespace and `-` separators are stripped before validation,
/// so `"943 476 5919"`, `"943-476-5919"`, and `"9434765919"` are
/// all equivalent inputs.
pub fn nhs(value: &str) -> bool {
    let digits: Vec<u32> = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-')
        .map(|c| c.to_digit(10))
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();
    if digits.len() != 10 {
        return false;
    }
    let total: u32 = digits.iter().zip((1..=10).rev()).map(|(d, w)| d * w).sum();
    total.is_multiple_of(11)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_known_numbers() {
        // Test number commonly used in NHS sandboxes.
        assert!(nhs("9434765919"));
        // Spaces and dashes are stripped.
        assert!(nhs("943 476 5919"));
        assert!(nhs("943-476-5919"));
    }

    #[test]
    fn invalid_check_digit() {
        // Wrong final digit fails the mod-11 check.
        assert!(!nhs("9434765918"));
        assert!(!nhs("9434765910"));
    }

    #[test]
    fn rejects_non_digit_payload() {
        // Embedded letters can never become a 10-digit checksum
        // input.
        assert!(!nhs("ABC4765919"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!nhs("123"));
        assert!(!nhs("12345678901"));
        assert!(!nhs(""));
    }
}
