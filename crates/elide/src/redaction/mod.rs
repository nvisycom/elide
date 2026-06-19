//! Redaction: the [`Operator`] contract and the strategies that apply it.
//!
//! Re-exports the core redaction vocabulary (the [`Operator`] /
//! [`ReversibleOperator`] traits, [`Redactions`], [`LeakProfile`],
//! [`Vault`], …) from [`elide_core::redaction`], and adds the shipped
//! [`operators`], the default [`InMemoryVault`], and the pseudonym
//! [`generator`]s. The [`Anonymizer`] engine that drives them lives at
//! the crate root.
//!
//! [`Operator`]: elide_core::redaction::Operator
//! [`ReversibleOperator`]: elide_core::redaction::ReversibleOperator
//! [`Redactions`]: elide_core::redaction::Redactions
//! [`LeakProfile`]: elide_core::redaction::LeakProfile
//! [`Vault`]: elide_core::redaction::Vault
//! [`Anonymizer`]: crate::Anonymizer

pub mod generator;
mod vault;

#[doc(inline)]
pub use elide_core::redaction::*;

pub use self::vault::InMemoryVault;
pub use crate::anonymizer::operators;
