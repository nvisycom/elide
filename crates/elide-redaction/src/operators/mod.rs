//! Concrete redaction operators.
//!
//! Each shipped [`Operator`] reads the entity's value (the slice a
//! [`DataReader`] produced) and returns a modality [`Replacement`].
//!
//! Text and tabular: [`Mask`], [`Replace`], and [`Pseudonymize`] (a
//! consistent synthetic value per entity, drawn from a [`Generator`] and
//! kept stable across mentions through a [`Vault`], so coreferent mentions
//! all read the same surrogate).
//!
//! Hashing (feature `sha2`): `Sha2Hash` replaces the value with a one-way
//! SHA-2 digest.
//!
//! Tabular (feature `tabular`): `DropRow`, `DropColumn` — structural drops
//! that remove a whole record or field rather than editing a cell.
//!
//! Image (feature `image`): `Blur`, `Pixelate`, `Blackbox`.
//!
//! Audio (feature `audio`): `Silence`, `Beep`.
//!
//! Reversible (feature `aes`): `AesEncrypt` (AES-256-GCM) replaces the
//! value with a ciphertext recoverable given the key.
//!
//! Cross-modality: [`Erase`] removes the entity in any modality, and
//! [`Keep`] passes it through unchanged.
//!
//! [`Operator`]: elide_core::operator::Operator
//! [`Replacement`]: elide_core::modality::Modality::Replacement
//! [`Vault`]: crate::vault::Vault
//! [`Generator`]: crate::generator::Generator
//! [`DataReader`]: elide_core::modality::DataReader

#[cfg(feature = "audio")]
mod beep;
#[cfg(feature = "image")]
mod blackbox;
#[cfg(feature = "image")]
mod blur;
#[cfg(feature = "tabular")]
mod drop_column;
#[cfg(feature = "tabular")]
mod drop_row;
#[cfg(feature = "aes")]
mod encrypt;
mod erase;
#[cfg(feature = "sha2")]
mod hash;
mod keep;
mod mask;
#[cfg(feature = "image")]
mod pixelate;
mod pseudonymize;
mod replace;
#[cfg(feature = "audio")]
mod silence;

/// Replace an entity with a plausible, locale-aware fake value.
#[cfg(feature = "fake")]
#[cfg_attr(docsrs, doc(cfg(feature = "fake")))]
pub use elide_fake::Fake;

#[cfg(feature = "audio")]
pub use self::beep::Beep;
#[cfg(feature = "image")]
pub use self::blackbox::Blackbox;
#[cfg(feature = "image")]
pub use self::blur::Blur;
#[cfg(feature = "tabular")]
pub use self::drop_column::DropColumn;
#[cfg(feature = "tabular")]
pub use self::drop_row::DropRow;
#[cfg(feature = "aes")]
#[cfg_attr(docsrs, doc(cfg(feature = "aes")))]
pub use self::encrypt::{AesEncrypt, AesKey, KEY_LEN};
pub use self::erase::Erase;
#[cfg(feature = "sha2")]
#[cfg_attr(docsrs, doc(cfg(feature = "sha2")))]
pub use self::hash::{Sha2Algorithm, Sha2Hash};
pub use self::keep::Keep;
pub use self::mask::Mask;
#[cfg(feature = "image")]
pub use self::pixelate::Pixelate;
pub use self::pseudonymize::Pseudonymize;
pub use self::replace::Replace;
#[cfg(feature = "audio")]
pub use self::silence::Silence;
