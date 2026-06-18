//! Concrete text redaction operators.
//!
//! The shipped [`Operator`]s for the [`Text`] modality, mirroring
//! Presidio's set: [`Mask`], [`Replace`], [`Redact`], [`Hash`], and
//! [`Keep`]. Each reads the entity's value (the slice a [`DataReader`]
//! produced) and returns a [`TextReplacement`].
//!
//! [`Operator`]: elide_core::redaction::Operator
//! [`Text`]: elide_core::modality::text::Text
//! [`Hash`]: struct@Hash
//! [`DataReader`]: elide_core::modality::DataReader
//! [`TextReplacement`]: elide_core::modality::text::TextReplacement

mod hash;
mod keep;
mod mask;
mod redact;
mod replace;

pub use self::hash::{Hash, HashAlgorithm};
pub use self::keep::Keep;
pub use self::mask::Mask;
pub use self::redact::Redact;
pub use self::replace::Replace;
