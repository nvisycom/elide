//! Nigeria — patterns scoped to NG jurisdictional formats.
//!
//! See `assets/PRESIDIO.md` for third-party attribution.

use crate::{__shipped_pattern as shipped_pattern, Regex};

shipped_pattern!(
    /// NIN — 11-digit National Identification Number with
    /// Verhoeff checksum.
    fn nin from "../../../assets/patterns/ng/identity/nin.toml"
);
shipped_pattern!(
    /// Vehicle registration — 3-letter LGA + 3 digits + 2-letter
    /// year/batch (current 2011+ format).
    fn vehicle_registration from "../../../assets/patterns/ng/vehicle/registration.toml"
);

/// Every NG-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Regex> {
    vec![nin(), vehicle_registration()]
}
