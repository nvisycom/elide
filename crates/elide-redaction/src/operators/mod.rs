//! Concrete redaction operators.
//!
//! Each shipped [`Operator`] reads the entity's value (the slice a
//! [`DataReader`] produced) and returns a modality [`Replacement`].
//!
//! Text and tabular: [`Mask`], [`Replace`], [`Truncate`] (physically drop
//! the middle of a value, distinct from [`Mask`]), and [`Pseudonymize`] (a
//! consistent synthetic value per entity, drawn from a [`Generator`] and
//! kept stable across mentions through a [`Vault`], so coreferent mentions
//! all read the same surrogate).
//!
//! Value reduction: [`Clamp`] collapses out-of-range numbers into a
//! (localized) bucket label; [`GeneralizeDate`] (feature `datetime`) reduces
//! a date/timestamp to a coarser ISO-8601 granularity. Both only apply to
//! values of their shape, so both are a [`TryOperator`] — a value they
//! can't parse is *declined* rather than erased by fiat. [`WithFallback`]
//! wraps such an operator with any other [`Operator`] to run when it
//! declines, so a caller composes their own treatment for the leftover
//! values.
//!
//! Hashing (feature `sha2`): [`Sha2Hash`] replaces the value with a one-way
//! SHA-2 digest. Keyed hashing (feature `hmac`): [`HmacHash`] replaces it
//! with a keyed HMAC-SHA-2 digest, whose key stays secret. Both pick a
//! width from the shared [`Sha2Algorithm`].
//!
//! Tabular (feature `tabular`): [`DropRow`], [`DropColumn`] — structural drops
//! that remove a whole record or field rather than editing a cell.
//!
//! Image (feature `image`): [`Blur`], [`Pixelate`], [`Blackbox`].
//!
//! Audio (feature `audio`): [`Silence`], [`Beep`].
//!
//! Reversible (feature `aes`): [`AesEncrypt`] (AES-256-GCM) replaces the
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

// Operators grouped by modality (private submodules; the shipped types are
// re-exported below so `operators::Mask` etc. stay flat).
#[cfg(feature = "audio")]
mod audio;
#[cfg(feature = "image")]
mod image;
#[cfg(feature = "tabular")]
mod tabular;
mod text;

// Cross-modality and shared operator infrastructure, not tied to one
// medium.
mod erase;
mod keep;
#[cfg(any(feature = "hmac", feature = "aes"))]
mod key_provider;
#[cfg(any(feature = "sha2", feature = "hmac"))]
mod sha2_algorithm;
mod with_fallback;

/// Replace an entity with a plausible, locale-aware fake value.
#[cfg(feature = "fake")]
#[cfg_attr(docsrs, doc(cfg(feature = "fake")))]
pub use elide_fake::Fake;

// Audio interval operators.
#[cfg(feature = "audio")]
pub use self::audio::{Beep, Silence};
// Cross-modality and shared.
pub use self::erase::Erase;
// Image region operators.
#[cfg(feature = "image")]
pub use self::image::{Blackbox, Blur, Pixelate};
pub use self::keep::Keep;
/// The key-supply abstraction shared by [`HmacHash`] and [`AesEncrypt`].
#[cfg(any(feature = "hmac", feature = "aes"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "hmac", feature = "aes"))))]
pub use self::key_provider::{KeyProvider, StaticKey};
/// The SHA-2 digest width shared by [`Sha2Hash`] and [`HmacHash`].
#[cfg(any(feature = "sha2", feature = "hmac"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "sha2", feature = "hmac"))))]
pub use self::sha2_algorithm::Sha2Algorithm;
// Tabular structural operators.
#[cfg(feature = "tabular")]
pub use self::tabular::{DropColumn, DropRow};
#[cfg(feature = "aes")]
#[cfg_attr(docsrs, doc(cfg(feature = "aes")))]
pub use self::text::AesEncrypt;
#[cfg(feature = "hmac")]
#[cfg_attr(docsrs, doc(cfg(feature = "hmac")))]
pub use self::text::HmacHash;
#[cfg(feature = "sha2")]
#[cfg_attr(docsrs, doc(cfg(feature = "sha2")))]
pub use self::text::Sha2Hash;
// Text (and tabular-cell) operators.
pub use self::text::{Clamp, Mask, Pseudonymize, Replace, Truncate};
#[cfg(feature = "datetime")]
#[cfg_attr(docsrs, doc(cfg(feature = "datetime")))]
pub use self::text::{DateGranularity, DateStyle, GeneralizeDate};
pub use self::with_fallback::{TryOperator, WithFallback};
