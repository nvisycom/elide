//! Singapore NRIC / FIN checksum validator.
//!
//! 9 characters: prefix letter + 7 digits + check letter.
//!
//! Prefix:
//! - `S` — Singapore citizen / PR born before 2000
//! - `T` — Singapore citizen / PR born 2000+
//! - `F` — foreigner (Long-Term Pass) issued before 2000
//! - `G` — foreigner issued 2000+
//! - `M` — foreigner issued 2022+ (introduced after the original
//!   NRIC spec; check digit uses a different table and offset).
//!
//! Algorithm: weighted sum of the 7 digits with
//! `[2, 7, 6, 5, 4, 3, 2]`, plus an offset of 4 for `T`/`G`
//! and 3 for `M`. The remainder mod 11 indexes into a
//! prefix-specific letter alphabet; for `M`, the table is read
//! at position `10 - r`.

const ST_TABLE: &[u8; 11] = b"JZIHGFEDCBA";
const FG_TABLE: &[u8; 11] = b"XWUTRQPNMLK";
const M_TABLE: &[u8; 11] = b"KLJNPQRTUWX";
const WEIGHTS: [u32; 7] = [2, 7, 6, 5, 4, 3, 2];

/// Return `true` when `value` is a valid 9-character NRIC/FIN.
pub fn nric(value: &str) -> bool {
    let chars: Vec<char> = value
        .trim()
        .chars()
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if chars.len() != 9 {
        return false;
    }
    let prefix = chars[0];
    if !chars[1..8].iter().all(|c| c.is_ascii_digit()) || !chars[8].is_ascii_uppercase() {
        return false;
    }
    let digits: String = chars[1..8].iter().collect();
    let mut sum: u32 = digits
        .chars()
        .zip(WEIGHTS)
        .map(|(c, w)| c.to_digit(10).unwrap() * w)
        .sum();
    sum += match prefix {
        'T' | 'G' => 4,
        'M' => 3,
        'S' | 'F' => 0,
        _ => return false,
    };
    let r = (sum % 11) as usize;
    let (table, idx) = match prefix {
        'S' | 'T' => (ST_TABLE, r),
        'F' | 'G' => (FG_TABLE, r),
        'M' => (M_TABLE, 10 - r),
        _ => return false,
    };
    table[idx] as char == chars[8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_s_prefix() {
        assert!(nric("S2740116C"));
    }

    #[test]
    fn accepts_t_prefix() {
        assert!(nric("T1234567J"));
    }

    #[test]
    fn accepts_f_prefix() {
        assert!(nric("F1234567N"));
    }

    #[test]
    fn accepts_g_prefix() {
        assert!(nric("G1234567X"));
    }

    #[test]
    fn accepts_m_prefix() {
        assert!(nric("M1234567K"));
    }

    #[test]
    fn accepts_lowercase_input() {
        assert!(nric("s2740116c"));
    }

    #[test]
    fn rejects_unknown_prefix() {
        assert!(!nric("A2740116C"));
    }

    #[test]
    fn rejects_wrong_check_letter() {
        assert!(!nric("S2740116D"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!nric("S274011C"));
        assert!(!nric("S27401166C"));
        assert!(!nric(""));
    }

    #[test]
    fn rejects_non_digit_body() {
        assert!(!nric("S274A116C"));
    }
}
