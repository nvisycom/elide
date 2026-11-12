//! Canada, patterns scoped to CA jurisdictional formats.
//!
//! See `assets/PRESIDIO.md` for third-party attribution.

use super::shipped_pattern;
use crate::Pattern;

shipped_pattern!(
    /// SIN, the 9-digit Canadian Social Insurance Number, with a Luhn checksum.
    ///
    /// Its first digit is in `[1-7, 9]`.
    fn sin from "../../../assets/patterns/ca/identity/sin.toml"
);
shipped_pattern!(
    /// Canadian postal code, `A1A 1A1` (Canada Post Address
    /// Standard letter alphabet).
    fn postal_code from "../../../assets/patterns/ca/contact/postal_code.toml"
);

/// Every CA-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Pattern> {
    vec![sin(), postal_code()]
}
