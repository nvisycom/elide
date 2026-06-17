//! Spain-specific post-match validators.
//!
//! Registered under the [`ValidatorRegistry::builtin`] set with
//! dotted names — `"es.nif"`, `"es.nie"`, `"es.cif"`.
//!
//! [`ValidatorRegistry::builtin`]: super::ValidatorRegistry::builtin

mod cif;
mod nie;
mod nif;

pub use self::cif::cif;
pub use self::nie::nie;
pub use self::nif::nif;
