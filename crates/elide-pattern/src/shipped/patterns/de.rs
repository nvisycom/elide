//! Germany — patterns scoped to DE jurisdictional formats.
//!
//! See `assets/PRESIDIO.md` for third-party attribution.

use crate::{__shipped_pattern as shipped_pattern, Regex};

shipped_pattern!(
    /// Betriebsstättennummer (BSNR) — 9-digit clinic/practice ID.
    fn bsnr from "../../../assets/patterns/de/health/bsnr.toml"
);
shipped_pattern!(
    /// Lebenslange Arztnummer (LANR) — 9-digit physician ID with
    /// KBV checksum.
    fn lanr from "../../../assets/patterns/de/health/lanr.toml"
);
shipped_pattern!(
    /// Krankenversichertennummer (KVNR) — statutory health
    /// insurance ID per §290 SGB V.
    fn health_insurance from "../../../assets/patterns/de/health/health_insurance.toml"
);
shipped_pattern!(
    /// Personalausweisnummer — nPA (post-2010 ICAO) and legacy
    /// `T`+8-digit ID card formats.
    fn id_card from "../../../assets/patterns/de/identity/id_card.toml"
);
shipped_pattern!(
    /// Reisepassnummer — ICAO Doc 9303 passport serial.
    fn passport from "../../../assets/patterns/de/identity/passport.toml"
);
shipped_pattern!(
    /// Rentenversicherungsnummer (RVNR/SVNR) — Deutsche
    /// Rentenversicherung pension/social-security ID per VKVV § 4.
    fn social_security from "../../../assets/patterns/de/identity/social_security.toml"
);
shipped_pattern!(
    /// Steueridentifikationsnummer (Steuer-IdNr) — 11-digit
    /// lifetime tax identifier with ISO 7064 Mod 11, 10 checksum.
    fn tax_id from "../../../assets/patterns/de/identity/tax_id.toml"
);
shipped_pattern!(
    /// Steuernummer — regional Finanzamt tax number.
    fn tax_number from "../../../assets/patterns/de/identity/tax_number.toml"
);
shipped_pattern!(
    /// Umsatzsteuer-Identifikationsnummer (USt-IdNr) — VAT
    /// identification number.
    fn vat_id from "../../../assets/patterns/de/identity/vat_id.toml"
);
shipped_pattern!(
    /// Führerscheinnummer — EU-harmonized post-2013 driving
    /// licence number.
    fn fuehrerschein from "../../../assets/patterns/de/identity/fuehrerschein.toml"
);
shipped_pattern!(
    /// Kfz-Kennzeichen — license plate (Unterscheidungszeichen +
    /// Erkennungszeichen).
    fn kfz from "../../../assets/patterns/de/vehicle/kfz.toml"
);
shipped_pattern!(
    /// Postleitzahl (PLZ) — 5-digit postal code.
    fn plz from "../../../assets/patterns/de/contact/plz.toml"
);
shipped_pattern!(
    /// Handelsregisternummer — court-registered company ID with
    /// `HRA`/`HRB` section prefix.
    fn handelsregister from "../../../assets/patterns/de/finance/handelsregister.toml"
);

/// Every DE-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Regex> {
    vec![
        bsnr(),
        lanr(),
        health_insurance(),
        id_card(),
        passport(),
        social_security(),
        tax_id(),
        tax_number(),
        vat_id(),
        fuehrerschein(),
        kfz(),
        plz(),
        handelsregister(),
    ]
}
