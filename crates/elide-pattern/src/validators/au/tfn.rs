//! Australian Tax File Number (TFN) validator.
//!
//! 9 digits issued by the Australian Taxation Office. Algorithm:
//! weighted sum with `[1, 4, 3, 7, 5, 8, 6, 9, 10]` over all 9
//! digits must be divisible by 11.

const WEIGHTS: [u32; 9] = [1, 4, 3, 7, 5, 8, 6, 9, 10];

/// Return `true` when `value` is a valid 9-digit TFN. Whitespace
/// and dash separators in the canonical `NNN NNN NNN` rendering
/// are stripped before validation.
pub fn tfn(value: &str) -> bool {
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
    let sum: u32 = digits.iter().zip(WEIGHTS).map(|(d, w)| d * w).sum();
    sum.is_multiple_of(11)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_tfn() {
        // 123 456 782 — ATO documented test value.
        assert!(tfn("123456782"));
    }

    #[test]
    fn accepts_with_separators() {
        assert!(tfn("123 456 782"));
        assert!(tfn("123-456-782"));
    }

    #[test]
    fn accepts_second_vector() {
        assert!(tfn("100000001"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(!tfn("123456789"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!tfn("12345678"));
        assert!(!tfn("1234567823"));
        assert!(!tfn(""));
    }
}
