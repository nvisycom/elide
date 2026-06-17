//! Polish REGON (Rejestr Gospodarki Narodowej) validator.
//!
//! 9 digits (entity-level) or 14 digits (unit-level). Both use
//! a weighted-sum mod-11 check, with the 9-digit weights
//! `[8, 9, 2, 3, 4, 5, 6, 7]` and the 14-digit weights
//! `[2, 4, 8, 5, 0, 9, 7, 3, 6, 1, 2, 4, 8]`. A computed value
//! of 10 means the REGON is invalid (never assigned).

const WEIGHTS_9: [u32; 8] = [8, 9, 2, 3, 4, 5, 6, 7];
const WEIGHTS_14: [u32; 13] = [2, 4, 8, 5, 0, 9, 7, 3, 6, 1, 2, 4, 8];

/// Return `true` when `value` is a valid REGON in either the
/// 9-digit or 14-digit form. Hyphen and space separators are
/// stripped before validation.
pub fn regon(value: &str) -> bool {
    let chars: Vec<char> = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-')
        .collect();
    if !chars.iter().all(|c| c.is_ascii_digit()) {
        return false;
    }
    match chars.len() {
        9 => valid_with(&chars, &WEIGHTS_9),
        14 => valid_with(&chars[..9], &WEIGHTS_9) && valid_with(&chars, &WEIGHTS_14),
        _ => false,
    }
}

fn valid_with(chars: &[char], weights: &[u32]) -> bool {
    let body = &chars[..weights.len()];
    let check = chars[weights.len()];
    let sum: u32 = body
        .iter()
        .zip(weights)
        .map(|(c, w)| c.to_digit(10).unwrap() * w)
        .sum();
    let computed = sum % 11 % 10;
    computed == check.to_digit(10).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_9() {
        // 123456785 — widely-quoted valid REGON.
        assert!(regon("123456785"));
    }

    #[test]
    fn accepts_canonical_14() {
        // 12345678512347 — extends the 9-digit base with a valid
        // 14-digit suffix check.
        assert!(regon("12345678512347"));
    }

    #[test]
    fn accepts_with_separators() {
        assert!(regon("123-456-785"));
    }

    #[test]
    fn rejects_wrong_checksum_9() {
        assert!(!regon("123456789"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!regon("12345678"));
        assert!(!regon("1234567890"));
        assert!(!regon(""));
    }

    #[test]
    fn rejects_non_digit() {
        assert!(!regon("12345678A"));
    }
}
