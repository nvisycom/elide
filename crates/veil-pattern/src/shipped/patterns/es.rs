//! Spain — patterns scoped to ES jurisdictional formats.
//!
//! See `assets/PRESIDIO.md` for third-party attribution.

use crate::{__shipped_pattern as shipped_pattern, Regex};

shipped_pattern!(
    /// NIF / DNI — 8-digit national ID + Mod 23 letter.
    fn nif from "../../../assets/patterns/es/identity/nif.toml"
);
shipped_pattern!(
    /// NIE — foreign-resident ID with `X`/`Y`/`Z` prefix.
    fn nie from "../../../assets/patterns/es/identity/nie.toml"
);
shipped_pattern!(
    /// Spanish passport — 3 letters + 6 digits.
    fn passport from "../../../assets/patterns/es/identity/passport.toml"
);
shipped_pattern!(
    /// CIF — company tax ID with entity-class letter + 7 digits +
    /// control char (digit or letter per class).
    fn cif from "../../../assets/patterns/es/finance/cif.toml"
);
shipped_pattern!(
    /// Código postal — 5-digit postal code (province 01-52 in
    /// the leading pair).
    fn codigo_postal from "../../../assets/patterns/es/contact/codigo_postal.toml"
);

/// Every ES-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Regex> {
    vec![nif(), nie(), passport(), cif(), codigo_postal()]
}
