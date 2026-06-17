//! Australia-specific post-match validators.
//!
//! Registered under the [`ValidatorRegistry::builtin`] set with
//! dotted names — `"au.abn"`, `"au.acn"`, `"au.medicare"`,
//! `"au.tfn"`.
//!
//! [`ValidatorRegistry::builtin`]: super::ValidatorRegistry::builtin

mod abn;
mod acn;
mod medicare;
mod tfn;

pub use self::abn::abn;
pub use self::acn::acn;
pub use self::medicare::medicare;
pub use self::tfn::tfn;
