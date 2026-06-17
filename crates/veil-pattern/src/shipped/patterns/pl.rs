//! Poland — patterns scoped to PL jurisdictional formats.
//!
//! See `assets/PRESIDIO.md` for third-party attribution.

use crate::{__shipped_pattern as shipped_pattern, Regex};

shipped_pattern!(
    /// PESEL — 11-digit personal ID with date + sex + serial +
    /// weighted-mod-10 check digit.
    fn pesel from "../../../assets/patterns/pl/identity/pesel.toml"
);
shipped_pattern!(
    /// NIP — 10-digit taxpayer identification number, mod-11
    /// weighted checksum.
    fn nip from "../../../assets/patterns/pl/finance/nip.toml"
);
shipped_pattern!(
    /// REGON — 9 or 14-digit company registry number, mod-11
    /// weighted checksum.
    fn regon from "../../../assets/patterns/pl/finance/regon.toml"
);
shipped_pattern!(
    /// Kod pocztowy — `NN-NNN` postal code (Poczta Polska,
    /// 1973-present).
    fn kod_pocztowy from "../../../assets/patterns/pl/contact/kod_pocztowy.toml"
);

/// Every PL-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Regex> {
    vec![pesel(), nip(), regon(), kod_pocztowy()]
}
