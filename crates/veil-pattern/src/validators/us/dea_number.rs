//! US DEA (Drug Enforcement Administration) registration number
//! checksum validator.
//!
//! See `assets/NOTICE.md` for third-party attribution.

/// Return `true` if `value` is a valid DEA registration number.
///
/// DEA numbers are 9 characters: two letters (the registration
/// type and the surname initial) followed by seven digits, where
/// the last digit is a checksum.
///
/// The check takes the odd-position digits `d1, d3, d5` and the
/// even-position digits `d2, d4, d6`, then verifies that
/// `(sum(odd) + 2 * sum(even)) % 10 == d7`.
///
/// Whitespace and `-` separators are stripped before validation.
pub fn dea_number(value: &str) -> bool {
    let cleaned: String = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-')
        .collect();
    if cleaned.len() != 9 {
        return false;
    }
    let mut chars = cleaned.chars();
    let first = chars.next().unwrap();
    let second = chars.next().unwrap();
    if !first.is_ascii_alphabetic() || !second.is_ascii_alphabetic() {
        return false;
    }
    let digits: Vec<u32> = chars
        .map(|c| c.to_digit(10))
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();
    if digits.len() != 7 {
        return false;
    }
    let sum_odd = digits[0] + digits[2] + digits[4];
    let sum_even = digits[1] + digits[3] + digits[5];
    let expected = (sum_odd + 2 * sum_even) % 10;
    expected == digits[6]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_known_dea_numbers() {
        // AB1234563: odd = 1+3+5 = 9, even = 2+4+6 = 12,
        // (9 + 24) % 10 = 3 → matches d7 = 3.
        assert!(dea_number("AB1234563"));
        // BC9876562: odd = 9+7+5 = 21, even = 8+6+6 = 20,
        // (21 + 40) % 10 = 1 → mismatch with d7 = 2. Let me pick
        // a passing one. AF3456788: odd = 3+5+7 = 15, even =
        // 4+6+8 = 18, (15 + 36) % 10 = 1 → mismatch d7 = 8.
        // Easier: BB0000000 → odd = 0+0+0 = 0, even = 0+0+0 = 0,
        // d7 = 0. Valid.
        assert!(dea_number("BB0000000"));
    }

    #[test]
    fn strips_separators() {
        assert!(dea_number("AB-12-34563"));
        assert!(dea_number("AB 12 34563"));
    }

    #[test]
    fn rejects_wrong_check_digit() {
        assert!(!dea_number("AB1234560"));
        assert!(!dea_number("AB1234565"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!dea_number("AB123"));
        assert!(!dea_number("AB12345630"));
        assert!(!dea_number(""));
    }

    #[test]
    fn rejects_non_letter_prefix() {
        assert!(!dea_number("123456789"));
        assert!(!dea_number("A21234563"));
    }
}
