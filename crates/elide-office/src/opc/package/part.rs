//! [`PartPath`]: a typed package-part path, and the [`PartRole`] /
//! [`PartClassifier`] seam a format supplies to tell the engine how each kind of
//! part is treated.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

use hipstr::HipStr;

use crate::opc::block::EmbeddingKind;
use crate::opc::names::Names;
use crate::opc::package::content_types::CONTENT_TYPES_PART;
use crate::opc::package::kind::PartKind;
use crate::opc::xml::attributes::TextAttribute;

/// A package-part path within an OOXML zip (e.g. `word/document.xml`,
/// `word/media/image1.png`).
///
/// A newtype so a part is never addressed by a bare `String`: extraction hands
/// these back on every [`Block`](crate::opc::Block), and a
/// [`Replacement`](crate::opc::Replacement) targets one. Held as a [`HipStr`]
/// so a path clones cheaply (short paths inline; longer ones share).
///
/// OPC part names are ASCII case-insensitive, so two paths are equal, hash
/// alike, and order by their names with ASCII letters folded:
/// `word/media/Logo.png` is the part a relationship to `media/logo.png`
/// targets. The path keeps the spelling it was made with, so a part read from
/// the archive is written back under its own name.
#[derive(Debug, Clone)]
pub(crate) struct PartPath(HipStr<'static>);

impl PartPath {
    /// The part at `path` within the package.
    pub(crate) fn new(path: impl Into<HipStr<'static>>) -> Self {
        Self(path.into())
    }

    /// The path as a string slice.
    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Whether the path names the part `name`, compared ASCII
    /// case-insensitively as OPC part names are.
    #[must_use]
    pub(crate) fn is(&self, name: &str) -> bool {
        self.as_str().eq_ignore_ascii_case(name)
    }

    /// The path's file extension: the text after the last `.` of the final path
    /// segment, as written, or `None` if the final segment has no `.`. Use
    /// [`has_extension`](Self::has_extension) for a case-insensitive check.
    ///
    /// The OPC content-types manifest keys on this extension, and it treats a
    /// leading-dot name as extension-only: `_rels/.rels` has extension `rels`
    /// (matching `<Default Extension="rels">`), so this does too.
    #[must_use]
    pub(crate) fn extension(&self) -> Option<&str> {
        self.as_str()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .rsplit_once('.')
            .map(|(_, ext)| ext)
    }

    /// Whether the final path segment has extension `ext`, compared
    /// case-insensitively (e.g. `has_extension("xml")` for `document.xml`).
    #[must_use]
    pub(crate) fn has_extension(&self, ext: &str) -> bool {
        self.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(ext))
    }

    /// Whether the path lies within the directory `dir` (a `/`-terminated or bare
    /// prefix), i.e. `dir/...`, compared ASCII case-insensitively.
    /// `in_dir("xl/media")` matches `xl/media/image1.png` but not
    /// `xl/media.xml`.
    #[must_use]
    pub(crate) fn in_dir(&self, dir: &str) -> bool {
        let dir = dir.strip_suffix('/').unwrap_or(dir);
        let path = self.as_str();
        path.get(..dir.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(dir))
            && path[dir.len()..].starts_with('/')
    }

    /// Whether this is an OPC relationships part: the package root `_rels/.rels`,
    /// or a `<dir>/_rels/<name>.rels` sidecar. Both live in a `_rels/` directory
    /// and end in `.rels`, which uniquely identifies the OPC relationships parts
    /// across every OOXML format. Like any part name, compared ASCII
    /// case-insensitively.
    #[must_use]
    pub(crate) fn is_relationships(&self) -> bool {
        self.has_extension("rels")
            && (self.is("_rels/.rels") || self.as_str().to_ascii_lowercase().contains("/_rels/"))
    }

    /// Whether this is the OPC content-types manifest, `[Content_Types].xml`.
    #[must_use]
    pub(crate) fn is_content_types(&self) -> bool {
        self.is(CONTENT_TYPES_PART)
    }

    /// The path's bytes with ASCII letters folded, as OPC compares part names.
    fn folded(&self) -> impl Iterator<Item = u8> + '_ {
        self.as_str().bytes().map(|b| b.to_ascii_lowercase())
    }
}

impl PartialEq for PartPath {
    fn eq(&self, other: &Self) -> bool {
        self.is(other.as_str())
    }
}

impl Eq for PartPath {}

impl Hash for PartPath {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // As `str` hashes: the bytes, then a terminator, so equal paths hash
        // alike whatever their case.
        for b in self.folded() {
            state.write_u8(b);
        }
        state.write_u8(0xff);
    }
}

impl PartialOrd for PartPath {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PartPath {
    fn cmp(&self, other: &Self) -> Ordering {
        self.folded().cmp(other.folded())
    }
}

impl AsRef<str> for PartPath {
    fn as_ref(&self) -> &str {
        self.as_str()
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
///
/// Whatever its role, an XML part's [text
/// attributes](PartClassifier::text_attributes) are redacted as well.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PartRole {
    /// Redact the element text (text/comment/CDATA events) of this XML part.
    ElementText,
    /// Redact the external-hyperlink `Target` attribute values of this
    /// relationships part.
    RelationshipTargets,
    /// A binary embedding (image, audio or video clip, embedded object)
    /// surfaced as bytes,
    /// tagged with what kind of embedding it is.
    Binary(EmbeddingKind),
    /// A document-property part (docProps/core.xml, app.xml): its named fields
    /// carry personal data (author, timestamps, company), so it is surfaced for
    /// whole-part replacement, but it is not prose, so it is never text-spliced.
    Property,
    /// A spreadsheet's cell part (a worksheet or the shared-string table): its
    /// cell text is redacted out of band through the cell model and the part
    /// replaced whole. Every other text it carries (a header or footer, a
    /// validation list, a formula, a phonetic reading) is redacted as element
    /// text.
    Cells,
    /// Structure carried through unchanged, save for its text attributes.
    Structure,
}

impl PartRole {
    /// The role of a part of `kind` that every format treats alike: comments,
    /// charts, SmartArt, and drawings carry element text, a relationships part
    /// its external hyperlink targets, an embedding is binary, the document
    /// properties a property part, and everything else structure. A format
    /// places its own main part and stories before falling back on this.
    pub(crate) fn common(kind: PartKind) -> Self {
        match kind {
            PartKind::Comments | PartKind::Chart | PartKind::Diagram | PartKind::Drawing => {
                Self::ElementText
            }
            PartKind::Relationships => Self::RelationshipTargets,
            PartKind::Embedding(kind) => Self::Binary(kind),
            PartKind::Properties => Self::Property,
            _ => Self::Structure,
        }
    }

    /// Whether a whole-part byte replacement may target this part: a binary
    /// embedding is replaced wholesale, and a property or cell part is rewritten
    /// out of band. A text part is only ever spliced, and a structure part
    /// (the main part, the manifest, styles) never replaced, so redacted bytes
    /// can never overwrite the package's structure.
    pub(crate) fn is_whole_part_replaceable(self) -> bool {
        matches!(self, Self::Binary(_) | Self::Property | Self::Cells)
    }
}

/// A format supplies this so the engine acts on parts without knowing the
/// format's schema: it maps what a part is ([`PartKind`], decided once from the
/// part's content type) to how the engine treats it.
pub(crate) trait PartClassifier {
    /// The role the engine applies to a part of `kind`.
    fn role(&self, kind: PartKind) -> PartRole;

    /// The attributes whose values are text a reader sees or the policy keeps,
    /// redacted in every XML part like element text.
    fn text_attributes(&self) -> &'static [TextAttribute] {
        &[]
    }

    /// The names the format holds to rules of their own, and the attributes
    /// that refer to them; each is among the
    /// [`text_attributes`](Self::text_attributes).
    fn names(&self) -> &'static Names {
        &Names::NONE
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

    /// Part names compare as OPC compares them, ASCII case-insensitively,
    /// while each path keeps its own spelling.
    #[test]
    fn paths_differing_only_in_case_are_one_part() {
        use std::collections::{BTreeSet, HashSet};

        let archive = PartPath::from("word/media/Logo.png");
        let target = PartPath::from("word/media/logo.PNG");
        assert_eq!(archive, target);
        assert_eq!(archive.cmp(&target), Ordering::Equal);
        assert_eq!(
            HashSet::from([archive.clone(), target.clone()]).len(),
            1,
            "equal paths hash alike"
        );
        assert_eq!(BTreeSet::from([archive.clone(), target]).len(), 1);
        assert_eq!(archive.as_str(), "word/media/Logo.png");
        assert_ne!(archive, PartPath::from("word/media/logo.png.bak"));
        assert!(PartPath::from("[content_types].XML").is_content_types());
        assert!(PartPath::from("_RELS/.rels").is_relationships());
        assert!(PartPath::from("word/_Rels/document.xml.rels").is_relationships());
        assert!(PartPath::from("CustomXml/item1.xml").in_dir("customXml"));
    }

    #[test]
    fn is_relationships_matches_root_and_sidecar_rels_only() {
        assert!(PartPath::from("_rels/.rels").is_relationships());
        assert!(PartPath::from("word/_rels/document.xml.rels").is_relationships());
        assert!(PartPath::from("xl/worksheets/_rels/sheet1.xml.rels").is_relationships());
        // A file merely ending in `.rels` but not in a `_rels/` dir is not one,
        // the loose `ends_with(".rels")` check would wrongly accept this.
        assert!(!PartPath::from("data/foo.rels").is_relationships());
        assert!(!PartPath::from("word/document.xml").is_relationships());
    }
}
