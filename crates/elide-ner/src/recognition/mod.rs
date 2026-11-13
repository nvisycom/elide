//! Recognizer layer: the [`NerRecognizer`] that drives any
//! NER backend and emits entities from the canonical spans it
//! returns.
//!
//! Implements [`Recognizer<Text>`] so it composes with the
//! rest of the platform through the same trait every other text
//! recognizer uses.
//!
//! [`Recognizer<Text>`]: elide_core::recognition::Recognizer
//! [`Text`]: elide_core::modality::text::Text

mod recognizer;

pub use self::recognizer::NerRecognizer;
