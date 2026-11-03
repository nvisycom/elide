//! Redaction: the "hide" engines and the strategies they apply.
//!
//! The [`Anonymizer`] / [`Deanonymizer`] engines, the shipped [`operators`],
//! the [`vault`] backing (the default [`InMemoryVault`]), and the pseudonym
//! [`generator`]s (from [`elide_redaction`]), plus the core operator contract
//! and the [`HiddenContentPolicy`] every codec decodes under, re-exported from
//! [`elide_core::redaction`].
//!
//! [`Anonymizer`]: crate::redaction::Anonymizer
//! [`Deanonymizer`]: crate::redaction::Deanonymizer
//! [`Operator`]: elide_core::redaction::Operator
//! [`HiddenContentPolicy`]: elide_core::redaction::HiddenContentPolicy
//! [`operators`]: crate::redaction::operators
//! [`vault`]: crate::redaction::vault
//! [`InMemoryVault`]: crate::redaction::vault::InMemoryVault
//! [`generator`]: crate::redaction::generator

// The core operator contract and the hidden-content policy every codec decodes
// under, re-surfaced through the redaction crate.
#[doc(inline)]
pub use elide_core::redaction::{
    Disposition, HiddenContent, HiddenContentKind, HiddenContentPolicy, LeakProfile, Operator,
    OperatorId, Redactions, ReversibleOperator,
};
#[doc(inline)]
pub use elide_redaction::{
    Anonymizer, Deanonymizer, MatchContext, Rule, generator, operators, vault,
};
