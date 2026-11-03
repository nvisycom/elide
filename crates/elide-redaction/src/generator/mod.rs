//! Synthetic-value generators for [`Pseudonymize`], keyed off the
//! [`Generator`] seam.
//!
//! [`Pseudonymize`]: crate::operators::Pseudonymize
//! [`Generator`]: elide_core::redaction::Generator

mod random;

pub use self::random::RandomToken;
