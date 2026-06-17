//! Korean Resident Registration Number (RRN) validator.
//!
//! 13 digits in the form `YYMMDD-GHIJKLX` per the Ministry of
//! the Interior and Safety spec. Pre-October 2020 numbers carry
//! a checksum X computed from the first 12 digits with the
//! weights `[2,3,4,5,6,7,8,9,2,3,4,5]`; the checksum equals
//! `(11 - sum mod 11) mod 10`. Post-October 2020 numbers carry
//! a random tail and pass structural checks only.

pub(super) const WEIGHTS: [u32; 12] = [2, 3, 4, 5, 6, 7, 8, 9, 2, 3, 4, 5];

/// Return `true` when `value` is a valid RRN. Hyphen separators
/// are stripped before validation.
pub fn rrn(value: &str) -> bool {
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
    let region = digits[7] * 10 + digits[8];
    if region > 95 {
        return false;
    }
    let sum: u32 = digits[..12].iter().zip(WEIGHTS).map(|(d, w)| d * w).sum();
    let check = (11 - (sum % 11)) % 10;
    check == digits[12]
}

pub(super) fn structural_ok(digits: &[u32]) -> bool {
    let month = digits[2] * 10 + digits[3];
    let day = digits[4] * 10 + digits[5];
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_rrn() {
        // 800101 (DOB 1980-01-01) + gender 1 + region 11 + 234 +
        // computed check digit 3.
        assert!(rrn("8001011112343"));
    }

    #[test]
    fn accepts_with_dash_separator() {
        assert!(rrn("800101-1112343"));
    }

    #[test]
    fn rejects_invalid_month() {
        assert!(!rrn("8013011112343"));
    }

    #[test]
    fn rejects_invalid_day() {
        assert!(!rrn("8001321112343"));
    }

    #[test]
    fn rejects_invalid_region() {
        assert!(!rrn("8001011962343"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(!rrn("8001011112340"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!rrn("80010111123"));
        assert!(!rrn(""));
    }
}
