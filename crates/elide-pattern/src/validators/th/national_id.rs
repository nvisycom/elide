//! Thai National Identification Number (เลขประจำตัวประชาชน)
//! validator.
//!
//! 13 digits issued by the Department of Provincial
//! Administration (กรมการปกครอง). The first digit encodes the
//! citizenship category (1-8); 0 is reserved. The 13th digit is
//! a weighted checksum with weights `[13, 12, 11, 10, 9, 8, 7,
//! 6, 5, 4, 3, 2]` over the first 12 digits; check digit equals
//! `(11 - sum mod 11) mod 10`.

const WEIGHTS: [u32; 12] = [13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2];

/// Return `true` when `value` is a valid 13-digit Thai NID.
/// Whitespace and `-` separators in the rendering `X-XXXX-XXXXX-XX-X`
/// are stripped before validation.
pub fn national_id(value: &str) -> bool {
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
    if !(1..=8).contains(&digits[0]) {
        return false;
    }
    let sum: u32 = digits[..12].iter().zip(WEIGHTS).map(|(d, w)| d * w).sum();
    let check = (11 - (sum % 11)) % 10;
    check == digits[12]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_nid() {
        // Body 123456789012 → check 1.
        assert!(national_id("1234567890121"));
    }

    #[test]
    fn accepts_second_vector() {
        assert!(national_id("1000000000009"));
    }

    #[test]
    fn accepts_with_separators() {
        assert!(national_id("1-2345-67890-12-1"));
        assert!(national_id("1 2345 67890 12 1"));
    }

    #[test]
    fn rejects_reserved_leading_zero() {
        assert!(!national_id("0234567890121"));
    }

    #[test]
    fn rejects_leading_nine() {
        // Category 9 is not assigned.
        assert!(!national_id("9234567890121"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(!national_id("1234567890120"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!national_id("123456789012"));
        assert!(!national_id("12345678901211"));
        assert!(!national_id(""));
    }

    #[test]
    fn rejects_non_digit() {
        assert!(!national_id("123456789012A"));
    }
}
