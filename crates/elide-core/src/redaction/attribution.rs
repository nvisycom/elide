//! [`Attribution`]: the author-supplied "why" behind a redaction.

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Author-supplied rationale for a redaction: the policy it enforces and an
/// optional human-readable reason.
///
/// Where the matched selection rule answers *which rule fired*, an
/// `Attribution` answers *under what authority* — a compliance clause, an
/// internal policy, a data-handling rule. A policy author attaches it to a
/// selection rule (the anonymizer's `because`); the anonymizer records it on
/// the entity's [`Redaction`] event so an audit can trace a change back to
/// the policy that demanded it.
///
/// [`Redaction`]: crate::entity::provenance::EventKind::Redaction
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Attribution {
    /// Stable policy / rule identifier (e.g. `"gdpr-art-17"`, `"pci-dss-3.4"`).
    pub policy_id: HipStr<'static>,
    /// Human-readable reason (e.g. `"right to erasure"`), when given.
    pub reason: Option<HipStr<'static>>,
}

impl Attribution {
    /// An attribution citing `policy_id`, with no reason.
    pub fn new(policy_id: impl Into<HipStr<'static>>) -> Self {
        Self {
            policy_id: policy_id.into(),
            reason: None,
        }
    }

    /// Attach a human-readable `reason`, consuming and returning `self`.
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<HipStr<'static>>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

impl<T: Into<HipStr<'static>>> From<T> for Attribution {
    /// A policy id with no reason, from anything that converts to a string.
    fn from(policy_id: T) -> Self {
        Self::new(policy_id)
    }
}
