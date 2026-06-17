//! Canada-specific post-match validators.
//!
//! Registered under the [`ValidatorRegistry::builtin`] set with
//! dotted names — `"ca.sin"`.
//!
//! [`ValidatorRegistry::builtin`]: super::ValidatorRegistry::builtin

mod sin;

pub use self::sin::sin;
