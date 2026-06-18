//! United States — patterns scoped to US jurisdictional formats.

use crate::{__shipped_pattern as shipped_pattern, Regex};

shipped_pattern!(
    /// US bank routing numbers (ABA RTN, mod-10 validated).
    fn bank_routing from "../../../assets/patterns/us/finance/bank_routing.toml"
);
shipped_pattern!(
    /// US Social Security numbers (AAA-GG-SSSS).
    fn ssn from "../../../assets/patterns/us/identity/ssn.toml"
);
shipped_pattern!(
    /// US driver's license numbers (state-shape union).
    fn drivers_license from "../../../assets/patterns/us/identity/drivers_license.toml"
);
shipped_pattern!(
    /// US passport numbers.
    fn passport from "../../../assets/patterns/us/identity/passport.toml"
);
shipped_pattern!(
    /// US ZIP and ZIP+4 postal codes.
    fn postal_code from "../../../assets/patterns/us/identity/postal_code.toml"
);
shipped_pattern!(
    /// US Individual Taxpayer Identification Number (ITIN).
    fn itin from "../../../assets/patterns/us/identity/itin.toml"
);
shipped_pattern!(
    /// US National Provider Identifier (NPI, Luhn-on-80840 validated).
    fn npi from "../../../assets/patterns/us/health/npi.toml"
);
shipped_pattern!(
    /// US Medicare Beneficiary Identifier (MBI).
    fn mbi from "../../../assets/patterns/us/health/mbi.toml"
);
shipped_pattern!(
    /// US generic bank account number (8-17 digits, no checksum).
    /// Very weak score; relies on the context-keyword boost.
    fn bank_account from "../../../assets/patterns/us/finance/bank_account.toml"
);
shipped_pattern!(
    /// US DEA registration number (medical license,
    /// checksum-validated).
    fn medical_license from "../../../assets/patterns/us/health/medical_license.toml"
);

/// Every US-scoped built-in pattern.
#[must_use]
pub fn all() -> Vec<Regex> {
    vec![
        bank_routing(),
        ssn(),
        drivers_license(),
        passport(),
        postal_code(),
        itin(),
        npi(),
        mbi(),
        bank_account(),
        medical_license(),
    ]
}
