//! South Korea — patterns scoped to KR jurisdictional formats.
//!
//! See `assets/PRESIDIO.md` for third-party attribution.

use crate::{__shipped_pattern as shipped_pattern, Regex};

shipped_pattern!(
    /// RRN — 13-digit Resident Registration Number with
    /// weighted Mod-11 checksum (pre-Oct 2020) or random tail.
    fn rrn from "../../../assets/patterns/kr/identity/rrn.toml"
);
shipped_pattern!(
    /// FRN — Foreigner Registration Number (RRN shape with
    /// gender/century digit 5-8 and (13 - sum) Mod-10 checksum).
    fn frn from "../../../assets/patterns/kr/identity/frn.toml"
);
shipped_pattern!(
    /// BRN — Business Registration Number with magic-keys
    /// Mod-10 checksum.
    fn brn from "../../../assets/patterns/kr/finance/brn.toml"
);
shipped_pattern!(
    /// Korean passport — current (`LDDDLDDDD`) and legacy
    /// (`LDDDDDDDD`) formats.
    fn passport from "../../../assets/patterns/kr/identity/passport.toml"
);
shipped_pattern!(
    /// Driver's license — 12-digit with region-code allowlist.
    fn driver_license from "../../../assets/patterns/kr/identity/driver_license.toml"
);

/// Every KR-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Regex> {
    vec![rrn(), frn(), brn(), passport(), driver_license()]
}
