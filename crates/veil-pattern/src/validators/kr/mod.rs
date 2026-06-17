//! South Korea-specific post-match validators.
//!
//! Registered under the [`ValidatorRegistry::builtin`] set with
//! dotted names — `"kr.rrn"`, `"kr.frn"`, `"kr.brn"`,
//! `"kr.driver_license"`.
//!
//! [`ValidatorRegistry::builtin`]: super::ValidatorRegistry::builtin

mod brn;
mod driver_license;
mod frn;
mod rrn;

pub use self::brn::brn;
pub use self::driver_license::driver_license;
pub use self::frn::frn;
pub use self::rrn::rrn;
