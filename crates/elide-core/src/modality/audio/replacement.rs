//! [`AudioReplacement`]: what an audio operator produces.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::modality::ModalityReplacement;

/// What an audio operator produces to hide an entity: an acoustic
/// treatment applied to the entity's time range.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum AudioReplacement {
    /// Replace the range with silence, preserving its duration so the
    /// timeline does not shift.
    Silenced,
    /// Cut the range out entirely, shortening the stream.
    Removed,
    /// Leave the range untouched. Lets a policy keep a tagged range while
    /// redacting everything else.
    Unchanged,
}

impl ModalityReplacement for AudioReplacement {}
