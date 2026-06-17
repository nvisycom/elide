//! Concrete text redaction operators.
//!
//! The shipped [`Operator`](veil_core::redaction::Operator)s for the
//! [`Text`](veil_core::modality::text::Text) modality, mirroring
//! Presidio's set: [`Mask`], [`Replace`], [`Redact`], [`Hash`](struct@Hash), and
//! [`Keep`]. Each reads the entity's value (the slice a
//! [`DataReader`](veil_core::modality::DataReader) produced) and
//! returns a [`TextReplacement`](veil_core::modality::text::TextReplacement).

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
