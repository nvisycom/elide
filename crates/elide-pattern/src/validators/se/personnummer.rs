//! Swedish Personnummer validator.
//!
//! Encoded as `YYMMDD-XXXX` (6-digit form) or `YYYYMMDD-XXXX`
//! (8-digit form, post-2000 cohort). Samordningsnummer adds 60
//! to the day field. The trailing 10 digits carry a Luhn
//! checksum.

use super::luhn::luhn10;

/// Return `true` when `value` is a valid personnummer. Hyphen
/// `-` and plus `+` separators (the latter marks a 100-year-old
/// cohort) are accepted; whitespace is ignored.
pub fn personnummer(value: &str) -> bool {
    let digits: Vec<u32> = value
        .chars()
        .filter(|c| c.is_ascii_digit())
        .map(|c| c.to_digit(10).unwrap())
        .collect();
    let extras = value
        .chars()
        .filter(|c| !c.is_ascii_digit() && !c.is_ascii_whitespace() && *c != '-' && *c != '+')
        .count();
    if extras > 0 || !matches!(digits.len(), 10 | 12) {
        return false;
    }
    let pnr10: Vec<u32> = digits.into_iter().rev().take(10).rev().collect();

    let month = pnr10[2] * 10 + pnr10[3];
    let raw_day = pnr10[4] * 10 + pnr10[5];
    let day = if raw_day >= 61 { raw_day - 60 } else { raw_day };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return false;
    }

    luhn10(&pnr10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_personnummer() {
        // 900101-1239 — 6-digit date form with valid Luhn.
        assert!(personnummer("9001011239"));
        assert!(personnummer("900101-1239"));
    }

    #[test]
    fn accepts_8_digit_form() {
        // 19900101-1239 — 12-digit form; Luhn applies to last 10.
        assert!(personnummer("199001011239"));
        assert!(personnummer("19900101-1239"));
    }

    #[test]
    fn accepts_plus_separator() {
        // `+` marks a 100-year-old cohort; structurally identical.
        assert!(personnummer("900101+1239"));
    }

    #[test]
    fn accepts_samordningsnummer() {
        // Day 61 = samordningsnummer for day 1.
        // 900161 + 1234 → compute matching Luhn.
        let body = "900161123";
        for last in 0..10 {
            let s = format!("{body}{last}");
            let digits: Vec<u32> = s.chars().map(|c| c.to_digit(10).unwrap()).collect();
            if luhn10(&digits) {
                assert!(personnummer(&s));
                return;
            }
        }
        panic!("no valid samordningsnummer found for body {body}");
    }

    #[test]
    fn rejects_invalid_date() {
        // Month 13.
        assert!(!personnummer("9013011230"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(!personnummer("9001011230"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!personnummer("900101123"));
        assert!(!personnummer(""));
    }
}
