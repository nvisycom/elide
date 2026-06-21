//! [`AlignmentMode`]: how a sub-word NER span snaps to character
//! boundaries.

use serde::{Deserialize, Serialize};

/// How a sub-word span snaps to character boundaries.
///
/// Mirrors spaCy `Doc.char_span(alignment_mode=...)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlignmentMode {
    /// Reject spans that don't land on a token boundary.
    Strict,
    /// Shrink the span to the next inner token boundary.
    Contract,
    /// Expand the span to the next outer token boundary. Default:
    /// favors recall over precision.
    #[default]
    Expand,
}
