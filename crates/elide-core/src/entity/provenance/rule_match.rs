//! [`RuleMatch`]: which kind of selection rule chose an operator.

use hipstr::HipStr;
#[cfg(feature = "schema")]
use schemars::JsonSchema;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::entity::LabelRef;

/// A serializable summary of *which selection rule* bound an operator to an
/// entity — the automatic "why" behind a redaction.
///
/// The anonymizer selects an operator by walking an ordered rule list and
/// taking the first match. That decision is recorded on the [`Redaction`]
/// event as a `RuleMatch` so an audit can see *why* this operator ran
/// ("matched label EMAIL_ADDRESS", "carried tag financial", "the fallback").
///
/// This is a summary, not the live rule: a predicate rule can't carry its
/// closure into provenance, so [`Predicate`] records only that a predicate
/// matched, not which one.
///
/// [`Redaction`]: crate::entity::provenance::EventKind::Redaction
/// [`Predicate`]: RuleMatch::Predicate
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub enum RuleMatch {
    /// Matched an exact label rule.
    Label(LabelRef),
    /// Matched a tag rule: the entity's label carries this tag.
    Tag(#[cfg_attr(feature = "schema", schemars(with = "String"))] HipStr<'static>),
    /// Matched an arbitrary predicate rule (the closure is not captured).
    Predicate,
    /// Matched the catch-all fallback.
    Fallback,
}
