//! Germany-specific post-match validators.
//!
//! Registered under the [`ValidatorRegistry::builtin`] set with
//! dotted names — `"de.bsnr"`, `"de.lanr"`, `"de.passport"`,
//! `"de.id_card"`, `"de.health_insurance"`, `"de.social_security"`,
//! `"de.tax_id"`, `"de.vat_id"`, `"de.plz"`.
//!
//! [`ValidatorRegistry::builtin`]: super::ValidatorRegistry::builtin

mod bsnr;
mod health_insurance;
mod icao;
mod id_card;
mod lanr;
mod passport;
mod plz;
mod social_security;
mod tax_id;
mod vat_id;

pub use self::bsnr::bsnr;
pub use self::health_insurance::health_insurance;
pub use self::id_card::id_card;
pub use self::lanr::lanr;
pub use self::passport::passport;
pub use self::plz::plz;
pub use self::social_security::social_security;
pub use self::tax_id::tax_id;
pub use self::vat_id::vat_id;
