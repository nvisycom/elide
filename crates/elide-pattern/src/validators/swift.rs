//! SWIFT / BIC code validator (ISO 9362).
//!
//! The regex matches any 8- or 11-character uppercase alphanumeric run,
//! which collides with ordinary 8- and 11-letter uppercase words
//! (`DECEMBER`, `PLEASURE`, a shouted street name). The one structural
//! constraint a BIC always satisfies is that characters 5-6 are an ISO
//! 3166-1 country code, so this validator rejects any match whose country
//! segment is not a real country. That removes the bulk of the false
//! positives; a residual word whose 5th-6th letters happen to spell a
//! valid country code (rare) still relies on a nearby `swift`/`bic`/`bank`
//! context keyword to clear the confidence threshold.

use elide_core::primitive::CountryCode;

/// Return `true` when `value` is BIC-shaped *and* its country segment
/// (characters 5-6) is a valid ISO 3166-1 alpha-2 country code.
pub fn swift(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 8 && bytes.len() != 11 {
        return false;
    }
    if !bytes
        .iter()
        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return false;
    }
    // Bank code (1-4) must be letters; country code (5-6) must be a real
    // ISO 3166-1 country.
    if !bytes[..4].iter().all(u8::is_ascii_uppercase) {
        return false;
    }
    let country = &value[4..6];
    CountryCode::from_alpha2(country).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_real_bics() {
        assert!(swift("DEUTDEFF")); // Deutsche Bank, 8-char
        assert!(swift("PKOPPLPW")); // PKO BP, Poland
        assert!(swift("NWBKGB2L")); // NatWest, with digit in location
        assert!(swift("DEUTDEFF500")); // 11-char with branch
    }

    #[test]
    fn rejects_uppercase_words_with_invalid_country_segment() {
        assert!(!swift("DECEMBER")); // cc = MB, not a country
        assert!(!swift("PLEASURE")); // cc = SU (former USSR), not current ISO
    }

    #[test]
    fn residual_valid_country_segment_passes_structurally() {
        // `JEZEWSKIEGO` has cc = WS (Samoa), a real country, so the
        // structural validator cannot reject it; context gating is what
        // keeps such a word from surfacing. This test documents the known
        // residual rather than asserting an unachievable rejection.
        assert!(swift("JEZEWSKIEGO"));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(!swift("DEUTDEF")); // 7
        assert!(!swift("DEUTDEFF5")); // 9
        assert!(!swift(""));
    }

    #[test]
    fn rejects_lowercase_and_symbols() {
        assert!(!swift("deutdeff"));
        assert!(!swift("DEUT-DEFF"));
    }
}
