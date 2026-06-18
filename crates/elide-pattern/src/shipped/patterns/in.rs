//! India — patterns scoped to IN jurisdictional formats.
//!
//! See `assets/PRESIDIO.md` for third-party attribution.

use crate::{__shipped_pattern as shipped_pattern, Regex};

shipped_pattern!(
    /// Aadhaar — 12-digit UIDAI ID with Verhoeff checksum and
    /// no-palindrome rule.
    fn aadhaar from "../../../assets/patterns/in/identity/aadhaar.toml"
);
shipped_pattern!(
    /// PAN — 10-char Permanent Account Number with entity-type
    /// letter at position 4.
    fn pan from "../../../assets/patterns/in/identity/pan.toml"
);
shipped_pattern!(
    /// GSTIN — 15-char Goods and Services Tax ID with base-36
    /// weighted check digit.
    fn gstin from "../../../assets/patterns/in/finance/gstin.toml"
);
shipped_pattern!(
    /// Indian passport — 8-char alphanumeric (letter + non-zero
    /// digit + 5 digits + non-zero digit).
    fn passport from "../../../assets/patterns/in/identity/passport.toml"
);
shipped_pattern!(
    /// EPIC voter ID — 10-char alphanumeric issued by the
    /// Election Commission of India.
    fn voter from "../../../assets/patterns/in/identity/voter.toml"
);
shipped_pattern!(
    /// Indian vehicle registration — state + RTO district +
    /// series + 4-digit serial.
    fn vehicle_registration from "../../../assets/patterns/in/vehicle/registration.toml"
);

/// Every IN-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Regex> {
    vec![
        aadhaar(),
        pan(),
        gstin(),
        passport(),
        voter(),
        vehicle_registration(),
    ]
}
