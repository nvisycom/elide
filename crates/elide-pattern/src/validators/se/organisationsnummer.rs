//! Swedish Organisationsnummer validator.
//!
//! 10 digits with the third digit ≥ 2 (Bolagsverket's rule that
//! distinguishes organisationsnummer from personnummer) plus a
//! Luhn checksum over all 10.

use super::luhn::luhn10;

/// Return `true` when `value` is a valid organisationsnummer.
/// Hyphen separator is accepted; whitespace is ignored.
pub fn organisationsnummer(value: &str) -> bool {
    let digits: Vec<u32> = value
        .chars()
        .filter(|c| c.is_ascii_digit())
        .map(|c| c.to_digit(10).unwrap())
        .collect();
    let extras = value
        .chars()
        .filter(|c| !c.is_ascii_digit() && !c.is_ascii_whitespace() && *c != '-')
        .count();
    if extras > 0 || digits.len() != 10 {
        return false;
    }
    if digits[2] < 2 {
        return false;
    }
    luhn10(&digits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_orgnr() {
        // 556677-1233 — public Bolagsverket example shape.
        assert!(organisationsnummer("5566771233"));
        assert!(organisationsnummer("556677-1233"));
    }

    #[test]
    fn accepts_second_vector() {
        assert!(organisationsnummer("5522200004"));
    }

    #[test]
    fn rejects_low_third_digit() {
        // Third digit 1 → looks like a personnummer, not an org.
        assert!(!organisationsnummer("5516771233"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(!organisationsnummer("5566771230"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!organisationsnummer("556677123"));
        assert!(!organisationsnummer(""));
    }
}
