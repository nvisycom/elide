//! Indian Permanent Account Number (PAN) validator.
//!
//! 10 chars in the structural format `AAAAA9999A`. The 4th
//! character encodes the entity type per the Income Tax
//! Department: `P` individual, `C` company, `H` HUF, `F` firm,
//! `A` AOP, `T` trust, `B` BOI, `L` local authority, `J`
//! artificial juridical, `G` government. No published checksum.

const ENTITY_TYPES: &str = "PCHFATBLJG";

/// Return `true` when `value` is a structurally valid PAN.
pub fn pan(value: &str) -> bool {
    let normalized = value.trim().to_ascii_uppercase();
    let chars: Vec<char> = normalized.chars().collect();
    if chars.len() != 10 {
        return false;
    }
    if !chars[..3].iter().all(|c| c.is_ascii_uppercase()) {
        return false;
    }
    if !ENTITY_TYPES.contains(chars[3]) {
        return false;
    }
    if !chars[4].is_ascii_uppercase() {
        return false;
    }
    if !chars[5..9].iter().all(|c| c.is_ascii_digit()) {
        return false;
    }
    chars[9].is_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_pan() {
        // ABCPK1234E — individual entity (`P` at position 4).
        assert!(pan("ABCPK1234E"));
    }

    #[test]
    fn accepts_lowercase_input() {
        assert!(pan("abcpk1234e"));
    }

    #[test]
    fn accepts_company_pan() {
        // ABCCD1234E — company entity (`C` at position 4).
        assert!(pan("ABCCD1234E"));
    }

    #[test]
    fn rejects_invalid_entity_type() {
        // `X` is not a valid entity type at position 4.
        assert!(!pan("ABCXK1234E"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!pan("ABCPK1234"));
        assert!(!pan("ABCPK1234EE"));
        assert!(!pan(""));
    }

    #[test]
    fn rejects_wrong_digit_section() {
        assert!(!pan("ABCPK12A4E"));
    }
}
