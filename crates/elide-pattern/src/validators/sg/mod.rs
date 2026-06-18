//! Singapore-specific post-match validators.
//!
//! Registered under the [`ValidatorRegistry::builtin`] set with
//! dotted names — `"sg.nric"`, `"sg.uen"`.
//!
//! [`ValidatorRegistry::builtin`]: super::ValidatorRegistry::builtin

mod nric;
mod uen;

pub use self::nric::nric;
pub use self::uen::uen;
