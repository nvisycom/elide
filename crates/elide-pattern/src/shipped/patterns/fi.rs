//! Finland, patterns scoped to FI jurisdictional formats.
//!
//! See `assets/PRESIDIO.md` for third-party attribution.

use super::shipped_pattern;
use crate::Pattern;

shipped_pattern!(
    /// HETU, Finnish personal identity code with mod-31 control
    /// character.
    fn hetu from "../../../assets/patterns/fi/identity/hetu.toml"
);

/// Every FI-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Pattern> {
    vec![hetu()]
}
