//! Thailand — patterns scoped to TH jurisdictional formats.
//!
//! Algorithm sourced from the Department of Provincial
//! Administration spec (not from the Presidio bundle).

use crate::{__shipped_pattern as shipped_pattern, Regex};

shipped_pattern!(
    /// National ID (เลขประจำตัวประชาชน) — 13-digit ID with
    /// weighted Mod-11 check digit; first digit 1-8.
    fn national_id from "../../../assets/patterns/th/identity/national_id.toml"
);
shipped_pattern!(
    /// Thai postal code (รหัสไปรษณีย์) — 5-digit Thailand Post
    /// code with province prefix 10-96.
    fn postal_code from "../../../assets/patterns/th/contact/postal_code.toml"
);

/// Every TH-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Regex> {
    vec![national_id(), postal_code()]
}
