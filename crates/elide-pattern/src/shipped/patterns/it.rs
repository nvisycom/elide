//! Italy — patterns scoped to IT jurisdictional formats.
//!
//! See `assets/PRESIDIO.md` for third-party attribution.

use crate::{__shipped_pattern as shipped_pattern, Regex};

shipped_pattern!(
    /// Codice Fiscale — 16-character personal tax/health ID with
    /// odd/even-mapping checksum, omocodia-aware.
    fn fiscal_code from "../../../assets/patterns/it/identity/fiscal_code.toml"
);
shipped_pattern!(
    /// Carta d'Identità — paper-based, CIE 2.0, and CIE 3.0
    /// renderings.
    fn identity_card from "../../../assets/patterns/it/identity/identity_card.toml"
);
shipped_pattern!(
    /// Italian passport — 2 letters + 7 digits (Polizia di Stato
    /// format).
    fn passport from "../../../assets/patterns/it/identity/passport.toml"
);
shipped_pattern!(
    /// Patente di guida — classic + Motorizzazione Civile
    /// `U1`-prefixed format.
    fn driving_licence from "../../../assets/patterns/it/identity/driving_licence.toml"
);
shipped_pattern!(
    /// Partita IVA (P.IVA) — 11-digit VAT identifier with
    /// Luhn-like checksum.
    fn vat_code from "../../../assets/patterns/it/finance/vat_code.toml"
);

/// Every IT-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Regex> {
    vec![
        fiscal_code(),
        identity_card(),
        passport(),
        driving_licence(),
        vat_code(),
    ]
}
