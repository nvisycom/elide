//! Geographic region primitives.
//!
//! Currently the [`CountryCode`] ISO 3166-1 newtype, used to scope
//! region-sensitive recognizers to the country whose data formats they
//! target.

mod code;

pub use self::code::CountryCode;
