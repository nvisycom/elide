//! Poland, patterns scoped to PL jurisdictional formats.
//!
//! See `assets/PRESIDIO.md` for third-party attribution.

use super::shipped_pattern;
use crate::Pattern;

shipped_pattern!(
    /// PESEL, the 11-digit Polish personal ID with an encoded date of birth.
    ///
    /// Date + sex + serial + weighted-mod-10 check digit.
    fn pesel from "../../../assets/patterns/pl/identity/pesel.toml"
);
shipped_pattern!(
    /// NIP, 10-digit taxpayer identification number, mod-11
    /// weighted checksum.
    fn nip from "../../../assets/patterns/pl/finance/nip.toml"
);
shipped_pattern!(
    /// REGON, 9 or 14-digit company registry number, mod-11
    /// weighted checksum.
    fn regon from "../../../assets/patterns/pl/finance/regon.toml"
);
shipped_pattern!(
    /// Kod pocztowy, `NN-NNN` postal code (Poczta Polska,
    /// 1973-present).
    fn kod_pocztowy from "../../../assets/patterns/pl/contact/kod_pocztowy.toml"
);
shipped_pattern!(
    /// NRB, the 26-digit Polish domestic bank account number.
    ///
    /// A Polish IBAN without the `PL` prefix, with an ISO 13616 mod-97
    /// checksum.
    fn nrb from "../../../assets/patterns/pl/finance/nrb.toml"
);

/// Every PL-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Pattern> {
    vec![pesel(), nip(), regon(), kod_pocztowy(), nrb()]
}
