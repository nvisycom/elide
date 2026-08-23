//! Redaction: the "hide" engines and the strategies they apply.
//!
//! The [`Anonymizer`] / [`Deanonymizer`] engines (from [`elide_redaction`]),
//! the shipped [`operators`], the [`vault`] backing (the default
//! [`InMemoryVault`]), and the pseudonym [`generator`]s (from
//! [`elide_operator`]), plus the core operator contract re-exported from
//! [`elide_core::operator`].
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
// The selection/apply engines live in `elide-redaction`; the shipped operators,
// the key vault, and the pseudonym generators live in `elide-operator`.
#[doc(inline)]
pub use elide_operator::{generator, operators, vault};
#[doc(inline)]
pub use elide_redaction::{Anonymizer, Deanonymizer, MatchContext, Rule};
