//! German Reisepass (passport) ICAO Doc 9303 check-digit
//! validator.
//!
//! 9-character serial; the trailing digit is the check digit
//! computed via 7-3-1 weighting over the first 8 characters.
//! Letters `A`, `B`, `D`, `E`, `I`, `O`, `Q`, `S`, `U` are
//! visually ambiguous and never appear in ICAO travel-document
//! serials — reject outright so a lucky checksum can't promote a
//! non-passport string.

use super::icao::mrz_check_digit;

const FORBIDDEN_LETTERS: &str = "ABDEIOQSU";

/// Return `true` when `value` is a 9-character German passport
/// serial whose final digit matches the ICAO Doc 9303 check.
pub fn passport(value: &str) -> bool {
    let trimmed = value.trim().to_ascii_uppercase();
    if trimmed.len() != 9 {
        return false;
    }
    let (serial, check) = trimmed.split_at(8);
    let Some(check_digit) = check.chars().next().and_then(|c| c.to_digit(10)) else {
        return false;
    };
    if serial.chars().any(|c| FORBIDDEN_LETTERS.contains(c)) {
        return false;
    }
    mrz_check_digit(serial) == Some(check_digit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_forbidden_letters() {
        // `B` is in the forbidden set.
        assert!(!passport("B1234567X"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!passport("C1234567"));
        assert!(!passport("C12345678X"));
        assert!(!passport(""));
    }

    #[test]
    fn rejects_non_digit_check() {
        assert!(!passport("C1234567X"));
    }

    #[test]
    fn accepts_with_valid_checksum() {
        // Serial `C0J9H58P`: compute mrz_check_digit manually.
        // C=12, 0=0, J=19, 9=9, H=17, 5=5, 8=8, P=25.
        // weights [7,3,1,7,3,1,7,3].
        // 84 + 0 + 19 + 63 + 51 + 5 + 56 + 75 = 353; 353 mod 10 = 3.
        assert!(passport("C0J9H58P3"));
    }

    #[test]
    fn rejects_invalid_checksum() {
        assert!(!passport("C0J9H58P4"));
    }
}
