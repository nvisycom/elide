//! Recognizer layer: the [`NerRecognizer`] that drives any
//! [`NerBackend`] backend and emits entities from the canonical spans it
//! returns.
//!
//! Implements [`Recognizer<Text>`] so it composes with the
//! rest of the platform through the same trait every other text
//! recognizer uses.
//!
//! [`NerBackend`]: crate::backend::NerBackend
//! [`Recognizer<Text>`]: elide_core::recognition::Recognizer
//! [`Text`]: elide_core::modality::Text

mod aggregation;
mod alignment;
mod recognizer;

pub use self::aggregation::AggregationStrategy;
pub use self::alignment::AlignmentMode;
pub use self::recognizer::{NerRecognizer, NerRecognizerBuilder};
