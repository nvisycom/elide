//! Australia — patterns scoped to AU jurisdictional formats.
//!
//! See `assets/PRESIDIO.md` for third-party attribution.

use crate::{__shipped_pattern as shipped_pattern, Regex};

shipped_pattern!(
    /// ABN — 11-digit Australian Business Number with mod-89
    /// weighted checksum.
    fn abn from "../../../assets/patterns/au/finance/abn.toml"
);
shipped_pattern!(
    /// ACN — 9-digit Australian Company Number with mod-10
    /// weighted checksum.
    fn acn from "../../../assets/patterns/au/finance/acn.toml"
);
shipped_pattern!(
    /// Medicare — 10-digit Australian Medicare card number
    /// (prefix 2-6, mod-10 weighted check).
    fn medicare from "../../../assets/patterns/au/health/medicare.toml"
);
shipped_pattern!(
    /// TFN — 9-digit Australian Tax File Number with mod-11
    /// weighted checksum.
    fn tfn from "../../../assets/patterns/au/identity/tfn.toml"
);

/// Every AU-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Regex> {
    vec![abn(), acn(), medicare(), tfn()]
}
