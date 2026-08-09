//! Text-modality operators.
//!
//! Each operates on the text slice under an entity. The value-editing
//! operators ([`Mask`], [`Replace`], [`Truncate`], [`Clamp`],
//! [`GeneralizeDate`], [`Sha2Hash`], [`HmacHash`], [`AesEncrypt`],
//! [`Pseudonymize`]) also implement [`Operator`]`<Tabular>` — a table cell is
//! text, so the same logic serves both, with a thin cell adapter. The
//! shipped operators are re-exported from [`operators`](super), the public
//! surface; this module is an internal grouping.
//!
//! [`Operator`]: elide_core::operator::Operator

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
pub use self::pseudonymize::{Pseudonymize, PseudonymizeKey};
pub use self::replace::Replace;
#[cfg(feature = "sha2")]
pub use self::sha2::Sha2Hash;
pub use self::truncate::Truncate;
