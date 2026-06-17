//! Spanish NIF / DNI checksum validator.
//!
//! 8 digits + control letter computed as `LETTERS[n mod 23]` over
//! the table `TRWAGMYFPDXBNJZSQVHLCKE` (Real Decreto 338/1990).
//! Older DNIs may be issued with a leading `0` truncated, so 7
//! digits + letter is also accepted; the modulo is taken over the
//! numeric value, so leading zeros don't matter.

pub(super) const LETTERS: &[u8; 23] = b"TRWAGMYFPDXBNJZSQVHLCKE";

/// Return `true` when `value` is a valid NIF (DNI) — 7 or 8
/// digits + Mod 23 control letter, with optional `-` separator.
pub fn nif(value: &str) -> bool {
    let normalized: String = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-')
        .collect();
    let chars: Vec<char> = normalized.chars().collect();
    if !matches!(chars.len(), 8 | 9) {
        return false;
    }
    let (digits, letter_char) = chars.split_at(chars.len() - 1);
    if !digits.iter().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let letter = letter_char[0].to_ascii_uppercase();
    if !letter.is_ascii_uppercase() {
        return false;
    }
    let number: u64 = digits.iter().collect::<String>().parse().unwrap_or(0);
    LETTERS[(number % 23) as usize] as char == letter
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_nif() {
        // 12345678 mod 23 = 14 → LETTERS[14] = 'Z'.
        assert!(nif("12345678Z"));
    }

    #[test]
    fn accepts_dash_separator() {
        assert!(nif("12345678-Z"));
    }

    #[test]
    fn accepts_7_digit_nif() {
        // 1234567 mod 23 = 19 → LETTERS[19] = 'L'.
        assert!(nif("1234567L"));
    }

    #[test]
    fn rejects_wrong_letter() {
        assert!(!nif("12345678A"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!nif("123456Z"));
        assert!(!nif("123456789Z"));
        assert!(!nif(""));
    }

    #[test]
    fn rejects_non_digit_body() {
        assert!(!nif("1234567AZ"));
    }
}
