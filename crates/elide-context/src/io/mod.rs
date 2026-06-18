//! Wiring between the [`Enhancer`] and the [`EntityRecognizer`]
//! pipeline.
//!
//! - [`Token`] / [`Tokens`] is the shared NLP token artifact the
//!   enhancer reads off `RecognizerInput.artifacts`.
//! - [`ContextEnhanced`] wraps any [`EntityRecognizer<Text>`] so
//!   the enhancer runs automatically after the inner recognizer's
//!   pass.
//!
//! All three types are re-exported at the crate root.
//!
//! [`Enhancer`]: crate::Enhancer
//! [`EntityRecognizer`]: elide_core::recognition::EntityRecognizer
//! [`EntityRecognizer<Text>`]: elide_core::recognition::EntityRecognizer

mod tokens;
mod wrapper;

pub use self::tokens::{Token, Tokens};
pub use self::wrapper::ContextEnhanced;
