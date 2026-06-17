//! Finland-specific post-match validators.
//!
//! Registered under the [`ValidatorRegistry::builtin`] set with
//! dotted names — `"fi.hetu"`.
//!
//! [`ValidatorRegistry::builtin`]: super::ValidatorRegistry::builtin

mod hetu;

pub use self::hetu::hetu;
