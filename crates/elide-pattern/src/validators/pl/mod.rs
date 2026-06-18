//! Poland-specific post-match validators.
//!
//! Registered under the [`ValidatorRegistry::builtin`] set with
//! dotted names — `"pl.pesel"`, `"pl.nip"`, `"pl.regon"`.
//!
//! [`ValidatorRegistry::builtin`]: super::ValidatorRegistry::builtin

mod nip;
mod pesel;
mod regon;

pub use self::nip::nip;
pub use self::pesel::pesel;
pub use self::regon::regon;
