//! [`PartPath`]: a typed package-part path, and the [`PartRole`] /
//! [`PartClassifier`] seam a format supplies to tell the engine how each part is
//! treated.

use std::fmt;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::opc::block::EmbeddingKind;

/// A package-part path within an OOXML zip (e.g. `word/document.xml`,
/// `word/media/image1.png`).
///
/// A newtype so a part is never addressed by a bare `String`: extraction hands
/// these back on every [`Block`](crate::opc::Block), and a
/// [`Replacement`](crate::opc::Replacement) targets one. Held as a [`HipStr`]
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

/// How the OPC engine treats a part, independent of any format's own richer
/// taxonomy.
///
/// A format's classifier maps its detailed part kinds down to one of these
/// roles, and the engine acts on the role alone — so the neutral core never
/// needs to know a `word/header2.xml` from a `word/comments.xml`, only that both
/// hold element text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartRole {
    /// Redact the element text (text/comment/CDATA events) of this XML part.
    ElementText,
    /// Redact the external-hyperlink `Target` attribute values of this
    /// relationships part.
    RelationshipTargets,
    /// A binary embedding (image, embedded object, font) surfaced as bytes,
    /// tagged with what kind of embedding it is.
    Binary(EmbeddingKind),
    /// Structure or metadata carried through unchanged.
    Structure,
}

impl PartRole {
    /// Whether this role yields redactable text — element text or relationship
    /// targets — so the engine reads it as a text-splice part.
    pub(crate) fn is_redactable(self) -> bool {
        matches!(self, Self::ElementText | Self::RelationshipTargets)
    }
}

/// A format supplies this so the engine can classify parts without knowing the
/// format's schema.
///
/// The engine calls [`role`](PartClassifier::role) to decide how to extract and
/// rewrite each part, and [`is_protected`](PartClassifier::is_protected) to
/// refuse a binary part replacement that would corrupt the package.
pub trait PartClassifier {
    /// The role the engine should apply to the part at `path`.
    fn role(&self, path: &PartPath) -> PartRole;

    /// Whether `path` is a structural part a binary part replacement must never
    /// overwrite (clobbering it would corrupt the package rather than redact
    /// it). The default protects nothing; a format overrides it for the parts
    /// whose bytes hold the package's structure.
    fn is_protected(&self, _path: &PartPath) -> bool {
        false
    }
}
