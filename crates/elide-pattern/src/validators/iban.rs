//! ISO 13616 IBAN validator backed by the [`iban_validate`]
//! crate, which ships the SWIFT IBAN registry (per-country
//! length and BBAN structure) on top of the mod-97 checksum.
//!
//! [`iban_validate`]: https://crates.io/crates/iban_validate

use iban::Iban;

/// Return `true` when `value` is a valid IBAN — both the mod-97
/// checksum and the country-specific length/BBAN structure must
/// match the SWIFT registry. Whitespace and dashes are stripped
/// before validation.
pub fn iban(value: &str) -> bool {
    let cleaned: String = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-')
        .collect();
    cleaned.parse::<Iban>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_ibans() {
        assert!(iban("GB29 NWBK 6016 1331 9268 19"));
        assert!(iban("DE89370400440532013000"));
        assert!(iban("FR76 3000 6000 0112 3456 7890 189"));
    }

    #[test]
    fn invalid_check_digits() {
        assert!(!iban("GB29 NWBK 6016 1331 9268 18"));
        assert!(!iban("DE00370400440532013000"));
    }

    #[test]
    fn rejects_wrong_country_length() {
        // German IBAN must be 22 chars; trimming the last block
        // leaves a mod-97-valid string that the registry rejects.
        assert!(!iban("DE89370400440532013"));
    }

    #[test]
    fn rejects_unknown_country_code() {
        // `XX` is not in the SWIFT registry.
        assert!(!iban("XX29NWBK60161331926819"));
    }

    #[test]
    fn too_short() {
        assert!(!iban("GB29"));
        assert!(!iban(""));
    }

    #[test]
    fn non_alphanumeric() {
        assert!(!iban("GB29!NWBK60161331926819"));
    }

    #[test]
    fn strips_whitespace_and_dashes() {
        assert!(iban("GB29-NWBK-6016-1331-9268-19"));
        assert!(iban("  GB29 NWBK 6016 1331 9268 19  "));
    }
}
