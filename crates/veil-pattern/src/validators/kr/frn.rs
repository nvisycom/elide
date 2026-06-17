//! Korean Foreigner Registration Number (FRN) validator.
//!
//! Same structure as the RRN but the gender/century digit (G)
//! is in `[5-8]` and the checksum formula is
//! `(13 - sum mod 11) mod 10`.

use super::rrn::{WEIGHTS, structural_ok};

/// Return `true` when `value` is a valid FRN.
pub fn frn(value: &str) -> bool {
    let digits: Vec<u32> = value
        .chars()
        .filter(|c| c.is_ascii_digit())
        .map(|c| c.to_digit(10).unwrap())
        .collect();
    let extras = value
        .chars()
        .filter(|c| !c.is_ascii_digit() && !c.is_ascii_whitespace() && *c != '-')
        .count();
    if digits.len() != 13 || extras > 0 {
        return false;
    }
    if !structural_ok(&digits) {
        return false;
    }
    if !(5..=8).contains(&digits[6]) {
        return false;
    }
    let region = digits[7] * 10 + digits[8];
    if region > 95 {
        return false;
    }
    let sum: u32 = digits[..12].iter().zip(WEIGHTS).map(|(d, w)| d * w).sum();
    let check = (13 - (sum % 11)) % 10;
    check == digits[12]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_frn() {
        // 900101 + gender 5 (1900s male foreigner) + region 11 +
        // 234 + computed check digit 4.
        assert!(frn("9001015112344"));
    }

    #[test]
    fn accepts_with_dash_separator() {
        assert!(frn("900101-5112344"));
    }

    #[test]
    fn rejects_non_foreigner_gender_digit() {
        // Gender 1 → not FRN, expects 5-8.
        assert!(!frn("9001011112344"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(!frn("9001015112340"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!frn("900101511234"));
        assert!(!frn(""));
    }
}
