//! German Steueridentifikationsnummer (Steuer-IdNr) checksum
//! validator.
//!
//! 11-digit lifetime tax identifier per Bundeszentralamt für
//! Steuern (BZSt). Check digit at pos 11 derives from pos 1–10
//! via ISO 7064 Mod 11, 10.
//!
//! Post-2016 BZSt rule: no digit may appear more than three
//! times within pos 1–10. Also rules out the all-identical-digit
//! degenerate case the pre-2016 rule forbade.

/// Return `true` when `value` is a valid 11-digit German tax ID
/// per BZSt.
pub fn tax_id(value: &str) -> bool {
    let digits: Vec<u32> = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .map(|c| c.to_digit(10))
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();
    if digits.len() != 11 {
        return false;
    }
    if digits[0] == 0 {
        return false;
    }

    // No digit appears more than 3 times in positions 1-10.
    let mut counts = [0u32; 10];
    for d in &digits[..10] {
        counts[*d as usize] += 1;
    }
    if counts.iter().any(|&c| c > 3) {
        return false;
    }

    // ISO 7064 Mod 11, 10.
    let mut product = 10u32;
    for d in digits.iter().take(10) {
        let mut total = (d + product) % 10;
        if total == 0 {
            total = 10;
        }
        product = (total * 2) % 11;
    }
    let mut check = 11 - product;
    if check == 10 {
        check = 0;
    }
    check == digits[10]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_known_test_vectors() {
        // BZSt-published example IdNr (used in documentation).
        assert!(tax_id("65929970489"));
        // ELSTER demo IdNr from the official Steuer-IdNr leaflet.
        assert!(tax_id("36574261809"));
    }

    #[test]
    fn rejects_leading_zero() {
        assert!(!tax_id("06592997048"));
    }

    #[test]
    fn rejects_wrong_check_digit() {
        assert!(!tax_id("65929970480"));
    }

    #[test]
    fn rejects_digit_repeated_four_times() {
        // Four `1`s in positions 1-10.
        assert!(!tax_id("11112345601"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!tax_id("123456789"));
        assert!(!tax_id("123456789012"));
        assert!(!tax_id(""));
    }
}
