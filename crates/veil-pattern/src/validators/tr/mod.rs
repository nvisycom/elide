//! Turkey-specific post-match validators.
//!
//! Registered under the [`ValidatorRegistry::builtin`] set with
//! dotted names — `"tr.tckn"`.
//!
//! [`ValidatorRegistry::builtin`]: super::ValidatorRegistry::builtin

mod tckn;

pub use self::tckn::tckn;
