//! United Kingdom — patterns scoped to UK jurisdictional formats.
//!
//! See `assets/NOTICE.md` for third-party attribution.

use crate::{__shipped_pattern as shipped_pattern, Regex};

shipped_pattern!(
    /// UK NHS numbers (10-digit, mod-11 validated).
    fn nhs from "../../../assets/patterns/uk/identity/nhs.toml"
);
shipped_pattern!(
    /// UK National Insurance numbers (NINO).
    fn nino from "../../../assets/patterns/uk/identity/nino.toml"
);
shipped_pattern!(
    /// UK driving licence numbers (DVLA 16-character).
    fn driving_licence from "../../../assets/patterns/uk/identity/driving_licence.toml"
);
shipped_pattern!(
    /// UK postcodes (BS7666 format, plus GIR 0AA).
    fn postcode from "../../../assets/patterns/uk/contact/postcode.toml"
);
shipped_pattern!(
    /// UK vehicle registration numbers (current, prefix, and
    /// suffix eras).
    fn vehicle_registration from "../../../assets/patterns/uk/vehicle/registration.toml"
);
shipped_pattern!(
    /// UK passport numbers (post-2015 format). Weak score; relies
    /// on the context-keyword boost.
    fn passport from "../../../assets/patterns/uk/identity/passport.toml"
);

/// Every UK-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Regex> {
    vec![
        nhs(),
        nino(),
        driving_licence(),
        postcode(),
        vehicle_registration(),
        passport(),
    ]
}
