//! German Personalausweis (national ID card) validator.
//!
//! Two formats coexist on issued cards in 2026:
//!
//! - **nPA** (neuer Personalausweis), issued since November 2010:
//!   9 alphanumeric characters, ICAO Doc 9303 charset
//!   (excludes A, B, D, E, I, O, Q, S, U) plus a trailing check
//!   digit. The check digit uses 7-3-1 weights — same as
//!   [`super::passport`].
//! - **Legacy** `T`-prefix card, issued before 2010: letter `T`
//!   followed by 8 digits. The trailing digit is part of the
//!   serial, not a checksum. Accepted at pattern confidence
//!   because there is no structural check to apply.

use super::icao::mrz_check_digit;

const FORBIDDEN_LETTERS: &str = "ABDEIOQSU";

/// Return `true` when `value` is a structurally-plausible
/// German Personalausweis number — either an nPA serial with a
/// valid ICAO Doc 9303 check digit, or a legacy `T` + 8-digit
/// number.
pub fn id_card(value: &str) -> bool {
    let trimmed = value.trim().to_ascii_uppercase();
    if trimmed.len() != 9 {
        return false;
    }

    // Legacy T-format: `T` followed by 8 digits. No checksum.
    if trimmed.starts_with('T') && trimmed[1..].chars().all(|c| c.is_ascii_digit()) {
        return true;
    }

    // nPA: ICAO 7-3-1 over first 8 chars, no forbidden letters.
    let (serial, check) = trimmed.split_at(8);
    let Some(check_digit) = check.chars().next().and_then(|c| c.to_digit(10)) else {
        return false;
    };
    if serial.chars().any(|c| FORBIDDEN_LETTERS.contains(c)) {
        return false;
    }
    mrz_check_digit(serial) == Some(check_digit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_legacy_t_format() {
        assert!(id_card("T22000124"));
        // No checksum to verify, so any T+8-digit string passes.
        assert!(id_card("T00000000"));
    }

    #[test]
    fn rejects_legacy_with_letter_payload() {
        assert!(!id_card("T2200012A"));
    }

    #[test]
    fn accepts_npa_with_valid_check() {
        // Serial `L01X00T44` — known nPA sample (legal text in
        // PassG references).
        assert!(id_card("L01X00T44"));
    }

    #[test]
    fn rejects_npa_with_forbidden_letter() {
        // `B` is in the forbidden ICAO charset.
        assert!(!id_card("LB1X00T44"));
    }

    #[test]
    fn rejects_npa_with_invalid_check() {
        assert!(!id_card("L01X00T45"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!id_card("L01X00T4"));
        assert!(!id_card("L01X00T440"));
        assert!(!id_card(""));
    }
}
