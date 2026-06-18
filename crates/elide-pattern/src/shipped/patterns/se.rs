//! Sweden — patterns scoped to SE jurisdictional formats.
//!
//! See `assets/PRESIDIO.md` for third-party attribution.

use crate::{__shipped_pattern as shipped_pattern, Regex};

shipped_pattern!(
    /// Personnummer — 10/12-digit personal identity number with
    /// date validity (incl. samordningsnummer) and Luhn checksum.
    fn personnummer from "../../../assets/patterns/se/identity/personnummer.toml"
);
shipped_pattern!(
    /// Organisationsnummer — 10-digit Bolagsverket company ID
    /// with third digit ≥ 2 and Luhn checksum.
    fn organisationsnummer from "../../../assets/patterns/se/finance/organisationsnummer.toml"
);
shipped_pattern!(
    /// Postnummer — 5-digit postal code in `XXX XX` rendering
    /// (Postnord standard).
    fn postnummer from "../../../assets/patterns/se/contact/postnummer.toml"
);

/// Every SE-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Regex> {
    vec![personnummer(), organisationsnummer(), postnummer()]
}
