//! German Lebenslange Arztnummer (LANR) checksum validator.
//!
//! 9-digit lifetime physician number. Check digit at position 7
//! derives from positions 1–6 via the KBV Arztnummern-Richtlinie:
//! sum of `[4, 9, 4, 9, 4, 9]`-weighted digits, then the complement
//! to 10. Positions 8–9 carry the physician's Fachgruppe and are
//! not part of the checksum.

/// Return `true` when `value` is a 9-digit LANR with a valid
/// position-7 check digit per KBV.
pub fn lanr(value: &str) -> bool {
    let digits: Vec<u32> = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .map(|c| c.to_digit(10))
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();
    if digits.len() != 9 {
        return false;
    }
    let total: u32 = digits
        .iter()
        .take(6)
        .zip([4, 9, 4, 9, 4, 9])
        .map(|(d, w)| d * w)
        .sum();
    let expected = (10 - total % 10) % 10;
    digits[6] == expected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worked_example() {
        // Per Presidio docstring: physician digits `123456` produce
        // check digit 6, so 123456601 is a valid LANR.
        assert!(lanr("123456601"));
    }

    #[test]
    fn additional_test_vectors() {
        // sum 234567 * [4,9,4,9,4,9] = 8+27+16+45+24+63 = 183; 183 mod 10 = 3; check = 7.
        assert!(lanr("234567701"));
        // sum 100000 * [4,9,4,9,4,9] = 4; 10-4 = 6.
        assert!(lanr("100000601"));
    }

    #[test]
    fn rejects_wrong_check_digit() {
        assert!(!lanr("123456701"));
        assert!(!lanr("123456501"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!lanr("12345670"));
        assert!(!lanr("1234566010"));
        assert!(!lanr(""));
    }

    #[test]
    fn rejects_non_digit() {
        assert!(!lanr("12345670A"));
    }
}
