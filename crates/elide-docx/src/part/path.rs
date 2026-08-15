//! [`PartPath`]: a typed package-part path.

use std::fmt;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::PartKind;

/// A package-part path within a DOCX zip (e.g. `word/document.xml`,
/// `word/media/image1.png`).
///
/// A newtype so a part is never addressed by a bare `String`: extraction hands
/// these back on every [`Block`](crate::block::Block), and a
/// [`Replacement`](crate::block::Replacement) targets one. Held as a [`HipStr`]
/// so a path clones cheaply (short paths inline; longer ones share).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(transparent))]
pub struct PartPath(HipStr<'static>);

impl PartPath {
    /// The part at `path` within the package.
    pub fn new(path: impl Into<HipStr<'static>>) -> Self {
        Self(path.into())
    }

    /// The path as a string slice.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// How this part is treated by extraction and redaction, from its path.
    pub fn kind(&self) -> PartKind {
        PartKind::of(self.as_str())
    }

    /// Whether this is a structural part a binary
    /// [`PartReplacement`](crate::block::PartReplacement) must never overwrite:
    /// the document body or the content-types manifest. Clobbering either would
    /// corrupt the package rather than redact it.
    pub fn is_protected(&self) -> bool {
        self.kind() == PartKind::Body || self.as_str() == "[Content_Types].xml"
    }
}

impl fmt::Display for PartPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for PartPath {
    fn from(path: &str) -> Self {
        Self::new(HipStr::from(path).into_owned())
    }
}

impl From<String> for PartPath {
    fn from(path: String) -> Self {
        Self::new(HipStr::from(path))
    }
}
