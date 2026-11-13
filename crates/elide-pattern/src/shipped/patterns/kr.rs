//! South Korea, patterns scoped to KR jurisdictional formats.
//!
//! See `assets/PRESIDIO.md` for third-party attribution.

use super::shipped_pattern;
use crate::Pattern;

shipped_pattern!(
    /// RRN, the 13-digit Korean Resident Registration Number.
    ///
    /// With a weighted Mod-11 checksum (pre-Oct 2020) or a random tail.
    fn rrn from "../../../assets/patterns/kr/identity/rrn.toml"
);
shipped_pattern!(
    /// FRN, the Korean Foreigner Registration Number, in the RRN shape.
    ///
    /// Its gender/century digit is 5-8, with a (13 - sum) Mod-10 checksum.
    fn frn from "../../../assets/patterns/kr/identity/frn.toml"
);
shipped_pattern!(
    /// BRN, Business Registration Number with magic-keys
    /// Mod-10 checksum.
    fn brn from "../../../assets/patterns/kr/finance/brn.toml"
);
shipped_pattern!(
    /// Korean passport, current (`LDDDLDDDD`) and legacy
    /// (`LDDDDDDDD`) formats.
    fn passport from "../../../assets/patterns/kr/identity/passport.toml"
);
shipped_pattern!(
    /// Driver's license, 12-digit with region-code allowlist.
    fn driver_license from "../../../assets/patterns/kr/identity/driver_license.toml"
);

/// Every KR-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Pattern> {
    vec![rrn(), frn(), brn(), passport(), driver_license()]
}
