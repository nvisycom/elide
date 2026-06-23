//! The shared NLP token artifact the [`Enhancer`] reads off the call's
//! `RecognizerContext.artifacts`.
//!
//! [`Token`] / [`Tokens`] are re-exported at the crate root.
//!
//! [`Enhancer`]: crate::Enhancer

mod tokens;

pub use self::tokens::{Token, Tokens};
