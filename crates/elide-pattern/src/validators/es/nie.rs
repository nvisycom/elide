//! Spanish NIE (Número de Identidad de Extranjero) validator.
//!
//! Format `[XYZ]` + 7 digits + control letter. The first letter
//! is replaced by its position in `XYZ` (`X→0`, `Y→1`, `Z→2`),
//! then the same Mod 23 check used by [`super::nif`] applies.

use super::nif::LETTERS;

/// Return `true` when `value` is a valid NIE — `X`, `Y`, or `Z`
/// prefix + 7 digits + Mod 23 control letter, with optional `-`.
pub fn nie(value: &str) -> bool {
    let normalized: String = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let chars: Vec<char> = normalized.chars().collect();
    if chars.len() != 9 {
        return false;
    }
    let prefix_pos = match chars[0] {
        'X' => 0,
        'Y' => 1,
        'Z' => 2,
        _ => return false,
    };
    if !chars[1..8].iter().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let letter = chars[8];
    if !letter.is_ascii_uppercase() {
        return false;
    }
    let body: String = std::iter::once(char::from(b'0' + prefix_pos))
        .chain(chars[1..8].iter().copied())
        .collect();
    let number: u64 = body.parse().unwrap_or(0);
    LETTERS[(number % 23) as usize] as char == letter
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_nie() {
        // X1234567 → 01234567 mod 23 = 12 → LETTERS[12] = 'L'.
        assert!(nie("X1234567L"));
    }

    #[test]
    fn accepts_dash_separator() {
        assert!(nie("X-1234567-L"));
    }

    #[test]
    fn accepts_y_prefix() {
        // Y1234567 → 11234567 mod 23 = 10 → LETTERS[10] = 'X'.
        assert!(nie("Y1234567X"));
    }

    #[test]
    fn accepts_z_prefix() {
        // Z1234567 → 21234567 mod 23 = 1 → LETTERS[1] = 'R'.
        assert!(nie("Z1234567R"));
    }

    #[test]
    fn rejects_wrong_prefix() {
        assert!(!nie("A1234567L"));
    }

    #[test]
    fn rejects_wrong_checksum() {
        assert!(!nie("X1234567A"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!nie("X123456L"));
        assert!(!nie(""));
    }
}
