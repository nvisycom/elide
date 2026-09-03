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

    /// The path's file extension: the text after the last `.` of the final path
    /// segment, as written, or `None` if the final segment has no `.`. Use
    /// [`has_extension`](Self::has_extension) for a case-insensitive check.
    ///
    /// The OPC content-types manifest keys on this extension, and it treats a
    /// leading-dot name as extension-only: `_rels/.rels` has extension `rels`
    /// (matching `<Default Extension="rels">`), so this does too.
    pub fn extension(&self) -> Option<&str> {
        self.as_str()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .rsplit_once('.')
            .map(|(_, ext)| ext)
    }

    /// Whether the final path segment has extension `ext`, compared
    /// case-insensitively (e.g. `has_extension("xml")` for `document.xml`).
    pub fn has_extension(&self, ext: &str) -> bool {
        self.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(ext))
    }

    /// Whether the path lies within the directory `dir` (a `/`-terminated or bare
    /// prefix), i.e. `dir/...`. `in_dir("xl/media")` matches `xl/media/image1.png`
    /// but not `xl/media.xml`.
    pub fn in_dir(&self, dir: &str) -> bool {
        let dir = dir.strip_suffix('/').unwrap_or(dir);
        self.as_str()
            .strip_prefix(dir)
            .is_some_and(|rest| rest.starts_with('/'))
    }

    /// Whether this is an OPC relationships part: the package root `_rels/.rels`,
    /// or a `<dir>/_rels/<name>.rels` sidecar. Both live in a `_rels/` directory
    /// and end in `.rels`, which uniquely identifies the OPC relationships parts
    /// across every OOXML format.
    pub fn is_relationships(&self) -> bool {
        self.has_extension("rels")
            && (self.as_str() == "_rels/.rels" || self.as_str().contains("/_rels/"))
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
/// roles, and the engine acts on the role alone, so the neutral core never
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
    /// Whether this role yields redactable text, element text or relationship
    /// targets, so the engine reads it as a text-splice part.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_is_the_suffix_of_the_final_segment() {
        // The raw suffix is returned as written; case-insensitive matching is
        // `has_extension`'s job.
        assert_eq!(
            PartPath::from("xl/media/image1.PNG").extension(),
            Some("PNG")
        );
        assert_eq!(PartPath::from("word/document.xml").extension(), Some("xml"));
        // A dotless segment has no extension, even under a dotted directory.
        assert_eq!(PartPath::from("a.b/plain").extension(), None);
        // A leading dot on the segment is treated as extension-only (OPC keys on
        // `rels` for `_rels/.rels`).
        assert_eq!(PartPath::from("_rels/.rels").extension(), Some("rels"));
    }

    #[test]
    fn has_extension_is_case_insensitive() {
        assert!(PartPath::from("xl/media/image1.PNG").has_extension("png"));
        assert!(PartPath::from("word/document.xml").has_extension("XML"));
        assert!(!PartPath::from("word/document.xml").has_extension("rels"));
    }

    #[test]
    fn in_dir_matches_only_a_directory_prefix() {
        let media = PartPath::from("xl/media/image1.png");
        assert!(media.in_dir("xl/media"));
        assert!(media.in_dir("xl/media/")); // trailing slash accepted
        assert!(media.in_dir("xl"));
        // A sibling file that merely shares the prefix text is not inside it.
        assert!(!PartPath::from("xl/media.xml").in_dir("xl/media"));
        assert!(!media.in_dir("ppt"));
    }

    #[test]
    fn is_relationships_matches_root_and_sidecar_rels_only() {
        assert!(PartPath::from("_rels/.rels").is_relationships());
        assert!(PartPath::from("word/_rels/document.xml.rels").is_relationships());
        assert!(PartPath::from("xl/worksheets/_rels/sheet1.xml.rels").is_relationships());
        // A file merely ending in `.rels` but not in a `_rels/` dir is not one ,
        // the loose `ends_with(".rels")` check would wrongly accept this.
        assert!(!PartPath::from("data/foo.rels").is_relationships());
        assert!(!PartPath::from("word/document.xml").is_relationships());
    }
}
