//! Finland — patterns scoped to FI jurisdictional formats.
//!
//! See `assets/PRESIDIO.md` for third-party attribution.

use crate::{__shipped_pattern as shipped_pattern, Regex};

shipped_pattern!(
    /// HETU — Finnish personal identity code with mod-31 control
    /// character.
    fn hetu from "../../../assets/patterns/fi/identity/hetu.toml"
);

/// Every FI-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Regex> {
    vec![hetu()]
}
