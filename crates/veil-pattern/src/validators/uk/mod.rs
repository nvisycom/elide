//! UK-specific post-match validators.
//!
//! Registered under the [`ValidatorRegistry::builtin`] set with
//! dotted names — `"uk.nhs"`, `"uk.nino"`,
//! `"uk.driving_licence"`, `"uk.vehicle_registration"`.
//!
//! [`ValidatorRegistry::builtin`]: super::ValidatorRegistry::builtin

mod driving_licence;
mod nhs;
mod nino;
mod vehicle_registration;

pub use self::driving_licence::driving_licence;
pub use self::nhs::nhs;
pub use self::nino::nino;
pub use self::vehicle_registration::vehicle_registration;
