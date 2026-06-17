//! Canadian Social Insurance Number (SIN) validator.
//!
//! 9 digits with a Luhn check digit over the first 8. Numbers
//! beginning with `0` or `8` are reserved by Employment and
//! Social Development Canada (ESDC) and never assigned.

/// Return `true` when `value` is a valid Canadian SIN. Whitespace
/// and dash separators in the canonical `NNN NNN NNN` /
/// `NNN-NNN-NNN` renderings are stripped before validation.
pub fn sin(value: &str) -> bool {
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
    if digits[0] == 0 || digits[0] == 8 {
        return false;
    }
    let mut sum: u32 = 0;
    for (i, d) in digits.iter().rev().enumerate() {
        if i.is_multiple_of(2) {
            sum += d;
        } else {
            let m = d * 2;
            sum += if m > 9 { m - 9 } else { m };
        }
    }
    sum.is_multiple_of(10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_sin() {
        // 123 456 782 — widely-quoted ESDC test value.
        assert!(sin("123456782"));
    }

    #[test]
    fn accepts_with_separators() {
        assert!(sin("123 456 782"));
        assert!(sin("123-456-782"));
    }

    #[test]
    fn accepts_second_vector() {
        assert!(sin("100000009"));
    }

    #[test]
    fn rejects_reserved_prefix() {
        // 0xxxxxxxx and 8xxxxxxxx are reserved.
        assert!(!sin("012345670"));
        assert!(!sin("812345674"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(!sin("123456789"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!sin("12345678"));
        assert!(!sin("1234567823"));
        assert!(!sin(""));
    }
}
