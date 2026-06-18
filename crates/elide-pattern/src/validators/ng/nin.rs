//! Nigerian National Identification Number (NIN) validator.
//!
//! 11 digits issued by the National Identity Management
//! Commission (NIMC). The last digit is a Verhoeff checksum
//! over the preceding 10.

use super::super::verhoeff::verhoeff;

/// Return `true` when `value` is a valid 11-digit NIN.
pub fn nin(value: &str) -> bool {
    let digits: String = value.chars().filter(|c| c.is_ascii_digit()).collect();
    let extras = value
        .chars()
        .filter(|c| !c.is_ascii_digit() && !c.is_ascii_whitespace() && *c != '-')
        .count();
    if digits.len() != 11 || extras > 0 {
        return false;
    }
    verhoeff(&digits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_nin() {
        // Body 1234567890 → Verhoeff check 2.
        assert!(nin("12345678902"));
    }

    #[test]
    fn accepts_second_vector() {
        assert!(nin("98765432102"));
    }

    #[test]
    fn accepts_with_separators() {
        assert!(nin("1234 5678 902"));
        assert!(nin("123-456-78902"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(!nin("12345678900"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!nin("1234567890"));
        assert!(!nin("123456789033"));
        assert!(!nin(""));
    }

    #[test]
    fn rejects_non_digit() {
        assert!(!nin("1234567890A"));
    }
}
