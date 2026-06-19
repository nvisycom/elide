//! Redaction: the [`Operator`] contract and the "hide" engine that
//! applies it.
//!
//! Re-exports the core redaction vocabulary (the [`Operator`] /
//! [`ReversibleOperator`] traits, [`Redactions`], [`LeakProfile`], …)
//! from [`elide_core::redaction`], and adds the toolkit's own
//! [`Anonymizer`] engine plus the shipped [`operators`].
//!
//! [`Operator`]: elide_core::redaction::Operator
//! [`ReversibleOperator`]: elide_core::redaction::ReversibleOperator
//! [`Redactions`]: elide_core::redaction::Redactions
//! [`LeakProfile`]: elide_core::redaction::LeakProfile

#[doc(inline)]
pub use elide_core::redaction::*;

pub use crate::anonymizer::{Anonymizer, operators};
