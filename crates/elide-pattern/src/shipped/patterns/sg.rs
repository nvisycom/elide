//! Singapore — patterns scoped to SG jurisdictional formats.
//!
//! See `assets/PRESIDIO.md` for third-party attribution.

use crate::{__shipped_pattern as shipped_pattern, Regex};

shipped_pattern!(
    /// NRIC / FIN — Singapore National Registration Identity
    /// Card / Foreign Identification Number with weighted Mod-11
    /// checksum.
    fn nric from "../../../assets/patterns/sg/identity/nric.toml"
);
shipped_pattern!(
    /// UEN — Unique Entity Number issued by ACRA (formats A, B,
    /// and C, each with its own checksum).
    fn uen from "../../../assets/patterns/sg/finance/uen.toml"
);
shipped_pattern!(
    /// Singapore postal code — 6-digit Singapore Post code.
    fn postal_code from "../../../assets/patterns/sg/contact/postal_code.toml"
);

/// Every SG-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Regex> {
    vec![nric(), uen(), postal_code()]
}
