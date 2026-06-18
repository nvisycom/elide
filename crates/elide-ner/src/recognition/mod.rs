//! Recognizer layer: the [`NerRecognizer`] that drives any
//! [`NerBackend`] backend, the [`NerModel`] normalization knobs it
//! applies to raw spans, and the [`LabelMap`] translation table
//! shared across backends.
//!
//! Implements [`Recognizer<Text>`] so it composes with the
//! rest of the platform through the same trait every other text
//! recognizer uses.
//!
//! [`NerBackend`]: crate::backend::NerBackend
//! [`Recognizer<Text>`]: elide_core::recognition::Recognizer
//! [`Text`]: elide_core::modality::Text

mod aggregation;
mod config;
mod recognizer;

pub use elide_core::recognition::LabelMap;

pub use self::config::{NerModel, NerModelBuilder};
pub use self::recognizer::{NerRecognizer, NerRecognizerBuilder};
