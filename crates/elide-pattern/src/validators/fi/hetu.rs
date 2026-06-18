//! Finnish Henkilötunnus (HETU) personal identity code
//! validator.
//!
//! 11 characters: 6-digit date, century separator, 3-digit
//! serial, and a control character. The century separator
//! encodes the century of birth (1800s, 1900s, 2000s) via the
//! sets documented by Digi- ja väestötietovirasto. The control
//! character is the index of `int(date concatenated with serial)
//! mod 31` into the alphabet `0123456789ABCDEFHJKLMNPRSTUVWXY`.

const CONTROL_TABLE: &[u8; 31] = b"0123456789ABCDEFHJKLMNPRSTUVWXY";

fn is_separator(c: char) -> bool {
    matches!(
        c,
        '+' | '-' | 'A' | 'B' | 'C' | 'D' | 'E' | 'F' | 'Y' | 'X' | 'W' | 'V' | 'U'
    )
}

/// Return `true` when `value` is a valid 11-character HETU.
pub fn hetu(value: &str) -> bool {
    let trimmed = value.trim().to_ascii_uppercase();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() != 11 {
        return false;
    }
    if !chars[..6].iter().all(|c| c.is_ascii_digit())
        || !is_separator(chars[6])
        || !chars[7..10].iter().all(|c| c.is_ascii_digit())
        || !chars[10].is_ascii_alphanumeric()
    {
        return false;
    }

    let day: u32 = chars[0..2].iter().collect::<String>().parse().unwrap();
    let month: u32 = chars[2..4].iter().collect::<String>().parse().unwrap();
    if !(1..=31).contains(&day) || !(1..=12).contains(&month) {
        return false;
    }

    let date_serial: String = chars[..6].iter().chain(chars[7..10].iter()).collect();
    let n: u64 = date_serial.parse().unwrap();
    CONTROL_TABLE[(n % 31) as usize] as char == chars[10]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_hetu() {
        // 010170-123F per CONTROL_TABLE[(010170123) mod 31].
        assert!(hetu("010170-123F"));
    }

    #[test]
    fn accepts_2000s_century() {
        // Same digits with century char `A` (2000s) → recomputed.
        // 010101A123 → date+serial 010101123 mod 31 = ?; compute
        // and assert what falls out.
        let n: u64 = 10_101_123;
        let expected = CONTROL_TABLE[(n % 31) as usize] as char;
        assert!(hetu(&format!("010101A123{expected}")));
    }

    #[test]
    fn rejects_invalid_separator() {
        assert!(!hetu("010170Z123F"));
    }

    #[test]
    fn rejects_invalid_date() {
        // Day 32.
        assert!(!hetu("320170-123F"));
        // Month 13.
        assert!(!hetu("011370-123F"));
    }

    #[test]
    fn rejects_wrong_control_character() {
        assert!(!hetu("010170-123G"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!hetu("010170-123"));
        assert!(!hetu("010170-123FX"));
        assert!(!hetu(""));
    }
}
