//! India-specific post-match validators.
//!
//! Registered under the [`ValidatorRegistry::builtin`] set with
//! dotted names — `"in.aadhaar"`, `"in.pan"`, `"in.gstin"`.
//!
//! [`ValidatorRegistry::builtin`]: super::ValidatorRegistry::builtin

mod aadhaar;
mod gstin;
mod pan;

pub use self::aadhaar::aadhaar;
pub use self::gstin::gstin;
pub use self::pan::pan;
