//! The [`OperatorId`] redaction-operator identity.

use std::fmt;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Identifies a redaction operator, for the redaction audit a higher
/// layer assembles.
///
/// As with [`RecognizerId`], the version is part of the identity so the
/// audit trail records which build of the operator ran. The version is
/// opaque text — the core attaches no ordering semantics to it.
///
/// [`RecognizerId`]: crate::recognition::RecognizerId
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct OperatorId {
    /// Stable operator name (e.g. `"mask"`, `"aes-gcm-encrypt"`).
    pub name: HipStr<'static>,
    /// The operator's version at the time it was applied.
    pub version: HipStr<'static>,
}

impl OperatorId {
    /// Construct an operator identifier.
    pub fn new(name: impl Into<HipStr<'static>>, version: impl Into<HipStr<'static>>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
        }
    }
}

impl fmt::Display for OperatorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.name, self.version)
    }
}
