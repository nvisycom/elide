//! Redaction: the [`Operator`] contract and the strategies that apply it.
//!
//! Re-exports the core redaction vocabulary (the [`Operator`] /
//! [`ReversibleOperator`] traits, [`Redactions`], [`LeakProfile`],
//! [`Vault`], …) from [`elide_core::operator`], and adds the shipped
//! [`operators`], the default [`InMemoryVault`], and the pseudonym
//! [`generator`]s. The [`Anonymizer`] engine that drives them lives at
//! the crate root.
//!
//! [`Operator`]: elide_core::operator::Operator
//! [`ReversibleOperator`]: elide_core::operator::ReversibleOperator
//! [`Redactions`]: elide_core::operator::Redactions
//! [`LeakProfile`]: elide_core::operator::LeakProfile
//! [`Vault`]: elide_core::operator::Vault
//! [`Anonymizer`]: crate::Anonymizer

pub mod generator;
#[cfg(feature = "crypto")]
pub mod key_provider;
mod vault;

#[doc(inline)]
pub use elide_core::operator::*;

pub use self::vault::InMemoryVault;
pub use crate::anonymizer::operators;
