//! US-specific post-match validators.
//!
//! Registered under the [`ValidatorRegistry::builtin`] set with
//! dotted names — `"us.ssn"`, `"us.aba_routing"`, etc.
//!
//! [`ValidatorRegistry::builtin`]: super::ValidatorRegistry::builtin

mod aba_routing;
mod dea_number;
mod npi;
mod postal_code;
mod ssn;

pub use self::aba_routing::aba_routing;
pub use self::dea_number::dea_number;
pub use self::npi::npi;
pub use self::postal_code::postal_code;
pub use self::ssn::ssn;
