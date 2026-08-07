//! [`Attribution`]: the author-supplied "why" behind a redaction.

use hipstr::HipStr;
#[cfg(feature = "schema")]
use schemars::JsonSchema;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Author-supplied rationale for a redaction: a policy name and an optional
/// description.
///
/// Where the matched selection rule answers *which rule fired*, an
/// `Attribution` answers *under what authority* — a compliance clause, an
/// internal policy, a data-handling rule. A policy author attaches it to a
/// selection rule (`Rule::because` in `elide-redaction`); the anonymizer
/// records it on the entity's [`Redaction`] event so an audit can trace a
/// change back to the policy that demanded it.
///
/// The `name` is the author's label for that policy (`"gdpr-art-17"`,
/// `"hipaa-safe-harbor"`, `"PII removal"`); an optional `description` adds
/// human context. Any stable machine identity a policy layer needs (a rule
/// UUID, a jurisdiction) is that layer's concern — it can encode it in the
/// name or carry it separately.
///
/// [`Redaction`]: crate::entity::provenance::EventKind::Redaction
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Attribution {
    /// The policy's name (e.g. `"gdpr-art-17"`, `"hipaa-safe-harbor"`).
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub name: HipStr<'static>,
    /// Human-readable description (e.g. `"right to erasure"`), when given.
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub description: Option<HipStr<'static>>,
}

impl Attribution {
    /// An attribution named `name`, with no description.
    pub fn new(name: impl Into<HipStr<'static>>) -> Self {
        Self {
            name: name.into(),
            description: None,
        }
    }

    /// Attach a human-readable `description`, consuming and returning `self`.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<HipStr<'static>>) -> Self {
        self.description = Some(description.into());
        self
    }
}

impl<T: Into<HipStr<'static>>> From<T> for Attribution {
    fn from(name: T) -> Self {
        Self::new(name)
    }
}
