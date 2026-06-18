//! Korean Business Registration Number (BRN) validator.
//!
//! 10 digits formatted as `AAA-BB-CCCCC`. Checksum uses magic
//! keys `[1, 3, 7, 1, 3, 7, 1, 3, 5]` over the first 9 digits,
//! with the 9th digit getting an extra `(digit * 5) / 10` term
//! added to the sum. Check digit = `(10 - sum mod 10) mod 10`.

const MAGIC: [u32; 9] = [1, 3, 7, 1, 3, 7, 1, 3, 5];

/// Return `true` when `value` is a valid 10-digit BRN. Hyphen
/// separators are stripped before validation.
pub fn brn(value: &str) -> bool {
    let digits: Vec<u32> = value
        .chars()
        .filter(|c| c.is_ascii_digit())
        .map(|c| c.to_digit(10).unwrap())
        .collect();
    let extras = value
        .chars()
        .filter(|c| !c.is_ascii_digit() && !c.is_ascii_whitespace() && *c != '-')
        .count();
    if digits.len() != 10 || extras > 0 {
        return false;
    }

    let mut total = 0u32;
    for i in 0..8 {
        total += digits[i] * MAGIC[i];
    }
    let last = digits[8] * MAGIC[8];
    total += (last / 10) + last;
    let check = (10 - total % 10) % 10;
    check == digits[9]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_brn() {
        // 123456789 + computed check digit 1.
        assert!(brn("1234567891"));
    }

    #[test]
    fn accepts_with_dash_separator() {
        assert!(brn("123-45-67891"));
    }

    #[test]
    fn accepts_second_vector() {
        // 219810378 + check digit 3.
        assert!(brn("2198103783"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(!brn("1234567890"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!brn("123456789"));
        assert!(!brn("12345678901"));
        assert!(!brn(""));
    }
}
