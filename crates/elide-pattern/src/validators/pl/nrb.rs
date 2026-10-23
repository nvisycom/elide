//! Polish NRB (Numer Rachunku Bankowego) validator.
//!
//! 26 digits: two check digits followed by a 24-digit BBAN (an
//! 8-digit bank/branch number and a 16-digit account number). The
//! NRB is the domestic rendering of a Polish IBAN with the `PL`
//! prefix dropped — bank statements routinely print it this way —
//! so it carries the same ISO 13616 check: rearrange as
//! `BBAN + "2521" + check` (2521 encodes `PL`) and require the
//! value mod 97 to equal 1.

/// Return `true` when `value` is a valid 26-digit Polish NRB.
/// Space and hyphen separators in the conventional
/// `CC BBBB BBBB AAAA AAAA AAAA AAAA` grouping are stripped before
/// validation.
pub fn nrb(value: &str) -> bool {
    // Strip Unicode whitespace, not just ASCII: the pattern's `\s` matches
    // `White_Space`, so a PDF-extracted NRB can carry a U+00A0 non-breaking
    // space between digit groups that `is_ascii_whitespace` would leave in.
    let cleaned: Vec<char> = value
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    if cleaned.len() != 26 || !cleaned.iter().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let digits: Vec<u8> = cleaned
        .iter()
        .map(|c| c.to_digit(10).unwrap() as u8)
        .collect();
    let check = &digits[..2];
    let bban = &digits[2..];

    // mod 97 over the rearranged string `BBAN + 2521 + check`, folded
    // digit by digit so the 30-digit value never needs a wide integer.
    // 2521 is the ISO 13616 encoding of the country code `PL`
    // (P = 25, L = 21).
    const COUNTRY: [u8; 4] = [2, 5, 2, 1];
    let remainder = bban
        .iter()
        .chain(COUNTRY.iter())
        .chain(check.iter())
        .fold(0u32, |acc, &d| (acc * 10 + u32::from(d)) % 97);
    remainder == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_nrb() {
        // A real statement-rendered NRB (PKO BP), mod-97 valid.
        assert!(nrb("43102011690000810208293534"));
    }

    #[test]
    fn accepts_with_separators() {
        assert!(nrb("43 1020 1169 0000 8102 0829 3534"));
        assert!(nrb("43-1020-1169-0000-8102-0829-3534"));
    }

    #[test]
    fn accepts_non_breaking_space_separators() {
        // PDF text extraction often emits U+00A0 between groups; the pattern's
        // `\s` matches it, so the validator must strip it too.
        assert!(nrb(
            "43\u{00a0}1020\u{00a0}1169\u{00a0}0000\u{00a0}8102\u{00a0}0829\u{00a0}3534"
        ));
    }

    #[test]
    fn rejects_wrong_check_digits() {
        // Flip the leading check digits; mod-97 no longer holds.
        assert!(!nrb("44102011690000810208293534"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!nrb("4310201169000081020829353")); // 25
        assert!(!nrb("431020116900008102082935341")); // 27
        assert!(!nrb(""));
    }

    #[test]
    fn rejects_non_digit() {
        assert!(!nrb("4310201169000081020829353X"));
    }
}
