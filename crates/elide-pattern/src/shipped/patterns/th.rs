//! Thailand, patterns scoped to TH jurisdictional formats.
//!
//! Algorithm sourced from the Department of Provincial
//! Administration spec (not from the Presidio bundle).

use crate::{__shipped_pattern as shipped_pattern, Pattern};

shipped_pattern!(
    /// Thai national ID (เลขประจำตัวประชาชน), 13 digits with a check digit.
    ///
    /// A weighted Mod-11 check digit; the first digit is 1-8.
    fn national_id from "../../../assets/patterns/th/identity/national_id.toml"
);
shipped_pattern!(
    /// Thai postal codes (รหัสไปรษณีย์), 5 digits with a province prefix.
    ///
    /// The Thailand Post province prefix is 10-96.
    fn postal_code from "../../../assets/patterns/th/contact/postal_code.toml"
);

/// Every TH-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Pattern> {
    vec![national_id(), postal_code()]
}
