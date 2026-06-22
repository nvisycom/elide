//! Concrete redaction operators.
//!
//! Each shipped [`Operator`] reads the entity's value (the slice a
//! [`DataReader`] produced) and returns a modality [`Replacement`].
//!
//! Text and tabular: [`Mask`], [`Replace`], [`Hash`], and
//! [`Pseudonymize`] (a consistent synthetic value per entity, drawn from a
//! [`Generator`] and kept stable across mentions through a [`Vault`], so
//! coreferent mentions all read the same surrogate).
//!
//! Image (feature `image`): `Blur`, `Pixelate`, `Blackbox`.
//!
//! Audio (feature `audio`): `Silence`.
//!
//! Cross-modality: [`Erase`] removes the entity in any modality, and
//! [`Keep`] passes it through unchanged.
//!
//! [`Operator`]: elide_core::redaction::Operator
//! [`Replacement`]: elide_core::modality::Modality::Replacement
//! [`Vault`]: elide_core::redaction::Vault
//! [`Generator`]: crate::redaction::generator::Generator
//! [`Hash`]: struct@Hash
//! [`DataReader`]: elide_core::modality::DataReader

#[cfg(feature = "image")]
mod blackbox;
#[cfg(feature = "image")]
mod blur;
mod erase;
mod hash;
mod keep;
mod mask;
#[cfg(feature = "image")]
mod pixelate;
mod pseudonymize;
mod replace;
#[cfg(feature = "audio")]
mod silence;

#[cfg(feature = "image")]
pub use self::blackbox::Blackbox;
#[cfg(feature = "image")]
pub use self::blur::Blur;
pub use self::erase::Erase;
pub use self::hash::{Hash, HashAlgorithm};
pub use self::keep::Keep;
pub use self::mask::Mask;
#[cfg(feature = "image")]
pub use self::pixelate::Pixelate;
pub use self::pseudonymize::Pseudonymize;
pub use self::replace::Replace;
#[cfg(feature = "audio")]
pub use self::silence::Silence;
