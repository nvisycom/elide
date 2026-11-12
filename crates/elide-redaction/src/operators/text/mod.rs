//! Text operators.
//!
//! Each operates on the text under an entity, through one implementation over
//! every [`TextRedactable`] modality: plain text, table cells, and metadata
//! fields. The shipped operators are re-exported from [`operators`](super),
//! the public surface; this module is an internal grouping.
//!
//! [`TextRedactable`]: elide_core::modality::TextRedactable

mod clamp;
#[cfg(feature = "aes")]
mod encrypt;
#[cfg(feature = "datetime")]
mod generalize_date;
#[cfg(feature = "hmac")]
mod hmac;
mod mask;
mod pseudonymize;
mod replace;
#[cfg(feature = "sha2")]
mod sha2;
mod truncate;

pub use self::clamp::Clamp;
#[cfg(feature = "aes")]
pub use self::encrypt::AesEncrypt;
#[cfg(feature = "datetime")]
pub use self::generalize_date::{DateGranularity, DateStyle, GeneralizeDate};
#[cfg(feature = "hmac")]
pub use self::hmac::HmacHash;
pub use self::mask::Mask;
pub use self::pseudonymize::{Pseudonymize, PseudonymizeKey, PseudonymizeSeed};
pub use self::replace::Replace;
#[cfg(feature = "sha2")]
pub use self::sha2::Sha2Hash;
pub use self::truncate::Truncate;
