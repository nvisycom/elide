//! Sweden-specific post-match validators.
//!
//! Registered under the [`ValidatorRegistry::builtin`] set with
//! dotted names — `"se.personnummer"`,
//! `"se.organisationsnummer"`.
//!
//! [`ValidatorRegistry::builtin`]: super::ValidatorRegistry::builtin

mod luhn;
mod organisationsnummer;
mod personnummer;

pub use self::organisationsnummer::organisationsnummer;
pub use self::personnummer::personnummer;
