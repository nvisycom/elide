//! Australian Company Number (ACN) validator.
//!
//! 9 digits issued by ASIC. Algorithm: weighted sum of the first
//! 8 digits with `[8, 7, 6, 5, 4, 3, 2, 1]`; check digit equals
//! `(10 - sum mod 10) mod 10`.

const WEIGHTS: [u32; 8] = [8, 7, 6, 5, 4, 3, 2, 1];

/// Return `true` when `value` is a valid 9-digit ACN. Whitespace
/// and dash separators in the canonical `NNN NNN NNN` rendering
/// are stripped before validation.
pub fn acn(value: &str) -> bool {
    let digits: Vec<u32> = value
        .chars()
        .filter(|c| c.is_ascii_digit())
        .map(|c| c.to_digit(10).unwrap())
        .collect();
    let extras = value
        .chars()
        .filter(|c| !c.is_ascii_digit() && !c.is_ascii_whitespace() && *c != '-')
        .count();
    if digits.len() != 9 || extras > 0 {
        return false;
    }
    let sum: u32 = digits[..8].iter().zip(WEIGHTS).map(|(d, w)| d * w).sum();
    let check = (10 - sum % 10) % 10;
    check == digits[8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_acn() {
        // Body 12345678 → check 0.
        assert!(acn("123456780"));
    }

    #[test]
    fn accepts_with_separators() {
        assert!(acn("123 456 780"));
        assert!(acn("123-456-780"));
    }

    #[test]
    fn accepts_second_vector() {
        // Body 00400000 → check 6.
        assert!(acn("004000006"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(!acn("123456789"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!acn("12345678"));
        assert!(!acn("1234567890"));
        assert!(!acn(""));
    }
}
