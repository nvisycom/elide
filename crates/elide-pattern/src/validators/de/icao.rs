//! ICAO Doc 9303 check-digit helpers shared by German document
//! validators ([`passport`], [`id_card`]).
//!
//! Both nPA (Personalausweis since 2010) and German
//! Reisepass document numbers carry a 7-3-1 weighted check
//! digit at position 9, computed over the first 8 alphanumeric
//! characters with letters mapped `A`=10, `B`=11, …, `Z`=35.
//!
//! ICAO also restricts the serial-charset to letters excluding
//! `A`, `B`, `D`, `E`, `I`, `O`, `Q`, `S`, `U` (visually
//! ambiguous). Callers verify that themselves through the regex
//! character class; this helper only computes the checksum.
//!
//! [`passport`]: super::passport
//! [`id_card`]: super::id_card

/// Compute the ICAO Doc 9303 check digit over an 8-character
/// alphanumeric serial. Returns `None` when a character is not
/// `0`–`9` or `A`–`Z`.
pub(super) fn mrz_check_digit(serial: &str) -> Option<u32> {
    if serial.len() != 8 {
        return None;
    }
    let weights = [7, 3, 1];
    let mut total: u32 = 0;
    for (i, c) in serial.chars().enumerate() {
        let value = if c.is_ascii_digit() {
            c.to_digit(10).unwrap()
        } else if c.is_ascii_uppercase() {
            (c as u32) - ('A' as u32) + 10
        } else {
            return None;
        };
        total += value * weights[i % 3];
    }
    Some(total % 10)
}
