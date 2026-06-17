//! Canada — patterns scoped to CA jurisdictional formats.
//!
//! See `assets/PRESIDIO.md` for third-party attribution.

use crate::{__shipped_pattern as shipped_pattern, Regex};

shipped_pattern!(
    /// SIN — 9-digit Social Insurance Number with Luhn checksum
    /// (first digit in `[1-7, 9]`).
    fn sin from "../../../assets/patterns/ca/identity/sin.toml"
);
shipped_pattern!(
    /// Canadian postal code — `A1A 1A1` (Canada Post Address
    /// Standard letter alphabet).
    fn postal_code from "../../../assets/patterns/ca/contact/postal_code.toml"
);

/// Every CA-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Regex> {
    vec![sin(), postal_code()]
}
