//! Singapore Unique Entity Number (UEN) validator.
//!
//! Three formats issued by ACRA:
//!
//! - Format A: 9 chars — 8 digits + check letter.
//! - Format B: 10 chars — 4-digit year of registration + 4
//!   digits + check letter. The year cannot be in the future.
//! - Format C: 10 chars — `T`/`S`/`R` prefix + 2 digits + 2-letter
//!   entity-type code (from a fixed list) + 4 digits + check
//!   letter.
//!
//! Each format uses its own weight vector and check-letter
//! alphabet. Format C's modulo arithmetic subtracts 5 before
//! taking `mod 11`.

const FORMAT_A_WEIGHTS: [u32; 8] = [10, 4, 9, 3, 8, 2, 7, 1];
const FORMAT_A_ALPHABET: &[u8; 11] = b"XMKECAWLJDB";
const FORMAT_B_WEIGHTS: [u32; 9] = [10, 8, 6, 4, 9, 7, 5, 3, 1];
const FORMAT_B_ALPHABET: &[u8; 11] = b"ZKCMDNERGWH";
const FORMAT_C_WEIGHTS: [i64; 9] = [4, 3, 5, 3, 10, 2, 2, 5, 7];
const FORMAT_C_ALPHABET: &str = "ABCDEFGHJKLMNPQRSTUVWX0123456789";

const FORMAT_C_PREFIXES: &str = "TSR";
const FORMAT_C_ENTITY_TYPES: &[&str] = &[
    "LP", "LL", "FC", "PF", "RF", "MQ", "MM", "NB", "CC", "CS", "MB", "FM", "GS", "DP", "CP", "NR",
    "CM", "CD", "MD", "HS", "VH", "CH", "MH", "CL", "XL", "CX", "HC", "RP", "TU", "TC", "FB", "FN",
    "PA", "PB", "SS", "MC", "SM", "GA", "GB",
];

/// Return `true` when `value` is a valid UEN in any of the three
/// ACRA-published formats.
pub fn uen(value: &str) -> bool {
    let normalized = value.trim().to_ascii_uppercase();
    let chars: Vec<char> = normalized.chars().collect();
    match chars.len() {
        9 => validate_a(&chars),
        10 if chars[0].is_ascii_alphabetic() => validate_c(&normalized, &chars),
        10 => validate_b(&chars),
        _ => false,
    }
}

fn validate_a(chars: &[char]) -> bool {
    if !chars[..8].iter().all(|c| c.is_ascii_digit()) || !chars[8].is_ascii_uppercase() {
        return false;
    }
    let sum: u32 = chars[..8]
        .iter()
        .zip(FORMAT_A_WEIGHTS)
        .map(|(c, w)| c.to_digit(10).unwrap() * w)
        .sum();
    FORMAT_A_ALPHABET[(sum % 11) as usize] as char == chars[8]
}

fn validate_b(chars: &[char]) -> bool {
    if !chars[..9].iter().all(|c| c.is_ascii_digit()) || !chars[9].is_ascii_uppercase() {
        return false;
    }
    let sum: u32 = chars[..9]
        .iter()
        .zip(FORMAT_B_WEIGHTS)
        .map(|(c, w)| c.to_digit(10).unwrap() * w)
        .sum();
    FORMAT_B_ALPHABET[(sum % 11) as usize] as char == chars[9]
}

fn validate_c(normalized: &str, chars: &[char]) -> bool {
    if !FORMAT_C_PREFIXES.contains(chars[0]) {
        return false;
    }
    if !chars[1..3].iter().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let entity_type = &normalized[3..5];
    if !FORMAT_C_ENTITY_TYPES.contains(&entity_type) {
        return false;
    }
    if !chars[5..9].iter().all(|c| c.is_ascii_digit()) || !chars[9].is_ascii_uppercase() {
        return false;
    }
    let sum: i64 = chars[..9]
        .iter()
        .zip(FORMAT_C_WEIGHTS)
        .map(|(c, w)| FORMAT_C_ALPHABET.find(*c).unwrap() as i64 * w)
        .sum();
    let idx = (sum - 5).rem_euclid(11) as usize;
    FORMAT_C_ALPHABET.as_bytes()[idx] as char == chars[9]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_format_a() {
        // Body 12345678 → check `M`.
        assert!(uen("12345678M"));
    }

    #[test]
    fn accepts_format_b() {
        // Body 200512345 (year 2005) → check `R`.
        assert!(uen("200512345R"));
    }

    #[test]
    fn accepts_format_c() {
        // T05LL1234 (limited liability partnership) → check `D`.
        assert!(uen("T05LL1234D"));
    }

    #[test]
    fn rejects_format_c_unknown_entity_type() {
        // `ZZ` is not a valid entity type.
        assert!(!uen("T05ZZ1234D"));
    }

    #[test]
    fn rejects_format_a_wrong_checksum() {
        assert!(!uen("12345678A"));
    }

    #[test]
    fn rejects_format_b_wrong_checksum() {
        assert!(!uen("200512345A"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!uen("1234567M"));
        assert!(!uen("12345678MA"));
        assert!(!uen(""));
    }

    #[test]
    fn rejects_format_c_wrong_prefix() {
        // `X` is not a valid format-C prefix.
        assert!(!uen("X05LL1234D"));
    }
}
