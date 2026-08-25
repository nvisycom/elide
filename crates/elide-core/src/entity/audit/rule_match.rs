//! [`RuleMatch`]: which kind of selection rule chose an operator.

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::hash::AuditHasher;
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
/// [`Redaction`]: crate::entity::audit::AuditKind::Redaction
/// [`Predicate`]: RuleMatch::Predicate
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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

impl RuleMatch {
    /// Fold this rule match's identifying bytes into `out`, for the audit hash.
    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        match self {
            Self::Label(label) => {
                out.byte(0);
                out.bytes(label.as_str().as_bytes());
            }
            Self::Tag(tag) => {
                out.byte(1);
                out.bytes(tag.as_bytes());
            }
            Self::Predicate => {
                out.byte(2);
            }
            Self::Fallback => {
                out.byte(3);
            }
        }
    }
}
