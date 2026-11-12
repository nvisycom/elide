//! Sweden, patterns scoped to SE jurisdictional formats.
//!
//! See `assets/PRESIDIO.md` for third-party attribution.

use crate::{__shipped_pattern as shipped_pattern, Pattern};

shipped_pattern!(
    /// Personnummer, the 10- or 12-digit Swedish personal identity number.
    ///
    /// With date validity (incl. samordningsnummer) and a Luhn checksum.
    fn personnummer from "../../../assets/patterns/se/identity/personnummer.toml"
);
shipped_pattern!(
    /// Organisationsnummer, the 10-digit Swedish Bolagsverket company ID.
    ///
    /// Its third digit is ≥ 2, with a Luhn checksum.
    fn organisationsnummer from "../../../assets/patterns/se/finance/organisationsnummer.toml"
);
shipped_pattern!(
    /// Postnummer, 5-digit postal code in `XXX XX` rendering
    /// (Postnord standard).
    fn postnummer from "../../../assets/patterns/se/contact/postnummer.toml"
);

/// Every SE-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Pattern> {
    vec![personnummer(), organisationsnummer(), postnummer()]
}
