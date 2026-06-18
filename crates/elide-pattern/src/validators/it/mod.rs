//! Italy-specific post-match validators.
//!
//! Registered under the [`ValidatorRegistry::builtin`] set with
//! dotted names — `"it.fiscal_code"`, `"it.vat_code"`.
//!
//! [`ValidatorRegistry::builtin`]: super::ValidatorRegistry::builtin

mod fiscal_code;
mod vat_code;

pub use self::fiscal_code::fiscal_code;
pub use self::vat_code::vat_code;
