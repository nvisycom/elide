//! Indian Goods and Services Tax Identification Number (GSTIN)
//! validator.
//!
//! 15 chars: state code (01-37) + 10-char PAN + 13th char
//! (registration sequence) + `Z` literal at position 14 + check
//! digit at position 15. The check digit uses a base-36 weighted
//! sum per GST Network spec.

const BASE36: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";

fn base36_value(c: char) -> Option<u32> {
    BASE36.iter().position(|&b| b == c as u8).map(|p| p as u32)
}

/// Return `true` when `value` is a valid 15-char GSTIN.
pub fn gstin(value: &str) -> bool {
    let normalized = value.trim().to_ascii_uppercase();
    let chars: Vec<char> = normalized.chars().collect();
    if chars.len() != 15 {
        return false;
    }
    if !chars[..2].iter().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let state: u32 = chars[..2].iter().collect::<String>().parse().unwrap();
    if !(1..=37).contains(&state) {
        return false;
    }
    if chars[13] != 'Z' {
        return false;
    }
    if !chars.iter().all(|c| c.is_ascii_alphanumeric()) {
        return false;
    }

    // GSTIN checksum: each position's base-36 value is multiplied
    // by a factor alternating 1, 2, 1, 2, …; if the product is
    // ≥ 36, sum its base-36 digits (i.e. quotient + remainder).
    // The sum mod 36 gives a number `c`; check digit = (36 - c) mod 36.
    let mut total = 0u32;
    for (i, ch) in chars[..14].iter().enumerate() {
        let v = base36_value(*ch).unwrap_or(0);
        let factor = if i % 2 == 0 { 1 } else { 2 };
        let p = v * factor;
        total += (p / 36) + (p % 36);
    }
    let check_value = (36 - (total % 36)) % 36;
    base36_value(chars[14]) == Some(check_value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_gstin() {
        // 27AAAPL1234C1ZE — Maharashtra state 27, PAN AAAPL1234C,
        // reg 1, Z marker, base-36 weighted check digit `E`.
        assert!(gstin("27AAAPL1234C1ZE"));
    }

    #[test]
    fn accepts_second_vector() {
        // 29ABCDE1234F1ZW — Karnataka state 29, check `W`.
        assert!(gstin("29ABCDE1234F1ZW"));
    }

    #[test]
    fn rejects_invalid_state_code() {
        assert!(!gstin("00AAAPL1234C1ZE"));
        assert!(!gstin("99AAAPL1234C1ZE"));
    }

    #[test]
    fn rejects_missing_z_marker() {
        assert!(!gstin("27AAAPL1234C1AE"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(!gstin("27AAAPL1234C1Z0"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!gstin("27AAAPL1234C1Z"));
        assert!(!gstin(""));
    }
}
