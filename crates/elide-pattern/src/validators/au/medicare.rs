//! Australian Medicare number validator.
//!
//! Card numbers are 10 or 11 digits where the first 9 form the
//! identifier (first digit in `[2..=6]`) followed by a check
//! digit; the trailing digit(s) encode the individual reference
//! and issue number (not part of the checksum).
//!
//! Algorithm: weighted sum of the first 8 digits with
//! `[1, 3, 7, 9, 1, 3, 7, 9]` mod 10 equals the 9th digit.

const WEIGHTS: [u32; 8] = [1, 3, 7, 9, 1, 3, 7, 9];

/// Return `true` when the first 9 digits of `value` form a
/// Medicare number whose checksum matches. Whitespace and dash
/// separators are stripped before validation. Trailing digits
/// (individual reference + issue) are accepted but ignored.
pub fn medicare(value: &str) -> bool {
    let digits: Vec<u32> = value
        .chars()
        .filter(|c| c.is_ascii_digit())
        .map(|c| c.to_digit(10).unwrap())
        .collect();
    let extras = value
        .chars()
        .filter(|c| !c.is_ascii_digit() && !c.is_ascii_whitespace() && *c != '-')
        .count();
    if !(9..=11).contains(&digits.len()) || extras > 0 {
        return false;
    }
    if !(2..=6).contains(&digits[0]) {
        return false;
    }
    let sum: u32 = digits[..8].iter().zip(WEIGHTS).map(|(d, w)| d * w).sum();
    sum % 10 == digits[8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_medicare() {
        // Body 22281236 → check 6.
        assert!(medicare("222812366"));
    }

    #[test]
    fn accepts_with_separators() {
        assert!(medicare("2228 12366"));
        assert!(medicare("2228 1236 6"));
    }

    #[test]
    fn accepts_with_individual_reference() {
        // 10-digit form: 9-digit Medicare + individual reference.
        assert!(medicare("2228123661"));
    }

    #[test]
    fn rejects_wrong_prefix() {
        // Body must start with 2-6; 1xxx is invalid.
        assert!(!medicare("122812366"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(!medicare("222812360"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!medicare("22281236"));
        assert!(!medicare("222812366111"));
        assert!(!medicare(""));
    }
}
