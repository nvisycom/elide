//! Turkey, patterns scoped to TR jurisdictional formats.
//!
//! See `assets/PRESIDIO.md` for third-party attribution.

use crate::{__shipped_pattern as shipped_pattern, Pattern};

shipped_pattern!(
    /// TCKN, 11-digit T.C. Kimlik No with two-step weighted
    /// checksum.
    fn tckn from "../../../assets/patterns/tr/identity/tckn.toml"
);
shipped_pattern!(
    /// Turkish license plates (plaka), with province codes 01-81.
    ///
    /// Space- and hyphen-separated renderings.
    fn license_plate from "../../../assets/patterns/tr/vehicle/license_plate.toml"
);
shipped_pattern!(
    /// Turkish posta kodu, 5-digit postal code (province
    /// prefix 01-81).
    fn posta_kodu from "../../../assets/patterns/tr/contact/posta_kodu.toml"
);

/// Every TR-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Pattern> {
    vec![tckn(), license_plate(), posta_kodu()]
}
