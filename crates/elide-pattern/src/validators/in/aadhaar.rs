//! Indian Aadhaar number validator.
//!
//! 12 digits issued by UIDAI. Structural rules: leading digit
//! ≥ 2 (UIDAI reserves 0xx and 1xx); the number must not be a
//! palindrome; Verhoeff checksum over all 12 digits.

use super::super::verhoeff::verhoeff;

/// Return `true` when `value` is a valid 12-digit Aadhaar.
/// Whitespace and `-`/`:` separators are stripped before
/// validation.
pub fn aadhaar(value: &str) -> bool {
    let digits: String = value.chars().filter(|c| c.is_ascii_digit()).collect();
    let extras = value
        .chars()
        .filter(|c| !c.is_ascii_digit() && !c.is_ascii_whitespace() && *c != '-' && *c != ':')
        .count();
    if digits.len() != 12 || extras > 0 {
        return false;
    }
    let first = digits.chars().next().unwrap().to_digit(10).unwrap();
    if first < 2 {
        return false;
    }
    let reversed: String = digits.chars().rev().collect();
    if reversed == digits {
        return false;
    }
    verhoeff(&digits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_aadhaar() {
        // 234123412346 — widely-quoted Verhoeff-valid test value.
        assert!(aadhaar("234123412346"));
    }

    #[test]
    fn accepts_with_separators() {
        assert!(aadhaar("2341 2341 2346"));
        assert!(aadhaar("2341-2341-2346"));
        assert!(aadhaar("2341:2341:2346"));
    }

    #[test]
    fn rejects_leading_digit_below_two() {
        assert!(!aadhaar("134123412346"));
        assert!(!aadhaar("034123412346"));
    }

    #[test]
    fn rejects_palindrome() {
        // Palindrome would be rejected even with valid checksum.
        assert!(!aadhaar("212121212121"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(!aadhaar("234123412340"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!aadhaar("23412341234"));
        assert!(!aadhaar("2341234123466"));
        assert!(!aadhaar(""));
    }
}
