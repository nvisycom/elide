//! Nigeria-specific post-match validators.
//!
//! Registered under the [`ValidatorRegistry::builtin`] set with
//! dotted names — `"ng.nin"`.
//!
//! [`ValidatorRegistry::builtin`]: super::ValidatorRegistry::builtin

mod nin;

pub use self::nin::nin;
