//! Redaction: the "hide" engines and the strategies they apply.
//!
//! The [`Anonymizer`] / [`Deanonymizer`] engines, the shipped [`operators`],
//! the [`vault`] backing (the default [`InMemoryVault`]), and the pseudonym
//! [`generator`]s, plus the core operator contract re-exported from
//! [`elide_core::operator`]. Re-exported from [`elide_redaction`].
//!
//! [`Anonymizer`]: crate::redaction::Anonymizer
//! [`Deanonymizer`]: crate::redaction::Deanonymizer
//! [`Operator`]: elide_core::operator::Operator
//! [`operators`]: crate::redaction::operators
//! [`vault`]: crate::redaction::vault
//! [`InMemoryVault`]: crate::redaction::vault::InMemoryVault
//! [`generator`]: crate::redaction::generator

// The core operator contract, re-surfaced through the redaction crate.
#[doc(inline)]
pub use elide_core::operator::{LeakProfile, Operator, OperatorId, Redactions, ReversibleOperator};
#[doc(inline)]
pub use elide_redaction::{Anonymizer, Deanonymizer, generator, operators, vault};
