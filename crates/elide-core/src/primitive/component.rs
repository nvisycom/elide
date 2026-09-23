//! [`ComponentId`]: the name + version of an analysis component.

use std::fmt;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Identifies an analysis component — a recognizer or an enricher — by name and
/// version.
///
/// Pairs a stable name with a free-form version string so the audit trail and
/// usage report record not just *which* component ran but *which build* of it:
/// a rerun against an updated ruleset or model is then distinguishable from the
/// original. The version is opaque text (a semver, a checkpoint hash, a ruleset
/// date); the core attaches no ordering or comparison semantics to it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ComponentId {
    /// Stable, human-readable component name (e.g. `"us-ssn-pattern"`).
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub name: HipStr<'static>,
    /// The component's version at the time it ran.
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub version: HipStr<'static>,
}

impl ComponentId {
    /// Construct a component identifier.
    pub fn new(name: impl Into<HipStr<'static>>, version: impl Into<HipStr<'static>>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
        }
    }
}

impl fmt::Display for ComponentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.name, self.version)
    }
}
