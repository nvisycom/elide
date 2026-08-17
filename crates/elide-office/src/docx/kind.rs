//! [`PartKind`]: the Word-specific classification of a package part, and its
//! mapping onto the neutral [`PartRole`] the OPC engine acts on.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::opc::{EmbeddingKind, PartPath, PartRole};

/// What a package part is in a WordprocessingML document, which determines how
/// it is handled.
///
/// The text-bearing parts ([`Body`](PartKind::Body), [`Header`](PartKind::Header),
/// …) are extracted and redacted the same way: the WordprocessingML stories via
/// the `w:t` run text model, and [`Chart`](PartKind::Chart) /
/// [`Diagram`](PartKind::Diagram) via the user-visible text of their own schemas.
/// [`Embedding`](PartKind::Embedding) parts are binary (images, embedded objects)
/// and are surfaced for redaction as bytes. Everything else is structure or
/// metadata carried through unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
#[non_exhaustive]
pub enum PartKind {
    /// The main document body (`word/document.xml`).
    Body,
    /// A page header (`word/header{n}.xml`).
    Header,
    /// A page footer (`word/footer{n}.xml`).
    Footer,
    /// Footnote text (`word/footnotes.xml`).
    Footnotes,
    /// Endnote text (`word/endnotes.xml`).
    Endnotes,
    /// Comment text (`word/comments.xml`).
    Comments,
    /// A glossary / building-block document and its headers and footers
    /// (`word/glossary/document.xml`, `word/glossary/header{n}.xml`, …).
    Glossary,
    /// Chart text: axis titles, data labels, captions (`word/charts/chart{n}.xml`).
    Chart,
    /// SmartArt / diagram text (`word/diagrams/data{n}.xml`).
    Diagram,
    /// A relationships part (`*/_rels/*.rels`). Structure, but its external
    /// hyperlink relationships carry user-visible target URLs (`mailto:`,
    /// `https://…`) that hold the same PII as the body, so its `Target`
    /// attribute values are redacted.
    Relationships,
    /// A binary embedding (image, embedded object, font).
    Embedding(EmbeddingKind),
    /// Document metadata (`docProps/*`).
    Metadata,
    /// Package structure or any part not otherwise classified (relationships,
    /// `[Content_Types].xml`, settings, styles, …).
    Other,
}

impl PartKind {
    /// Classify `part` from its package path.
    pub fn of(part: &PartPath) -> Self {
        let path = part.as_str();
        // Text-bearing WordprocessingML parts.
        if path == "word/document.xml" {
            return Self::Body;
        }
        // Glossary story parts: the document and its own headers and footers.
        if path == "word/glossary/document.xml"
            || Self::is_numbered(path, "word/glossary/header", ".xml")
            || Self::is_numbered(path, "word/glossary/footer", ".xml")
        {
            return Self::Glossary;
        }
        if Self::is_numbered(path, "word/header", ".xml") {
            return Self::Header;
        }
        if Self::is_numbered(path, "word/footer", ".xml") {
            return Self::Footer;
        }
        if path == "word/footnotes.xml" {
            return Self::Footnotes;
        }
        if path == "word/endnotes.xml" {
            return Self::Endnotes;
        }
        if path == "word/comments.xml" {
            return Self::Comments;
        }
        // Chart and diagram parts carry user-visible text under their own
        // schemas; the extractor reads their text/comment/CDATA events too.
        if Self::is_numbered(path, "word/charts/chart", ".xml") {
            return Self::Chart;
        }
        if Self::is_numbered(path, "word/diagrams/data", ".xml") {
            return Self::Diagram;
        }
        // Relationships parts carry external hyperlink targets that hold user PII.
        if part.is_relationships() {
            return Self::Relationships;
        }
        if let Some(kind) = embedding_of(part) {
            return Self::Embedding(kind);
        }
        if path.starts_with("docProps/") {
            return Self::Metadata;
        }
        Self::Other
    }

    /// The neutral [`PartRole`] the OPC engine applies to this kind: text-bearing
    /// stories map to element text, a relationships part to relationship targets,
    /// an embedding to binary, and everything else to structure.
    pub(crate) fn role(self) -> PartRole {
        match self {
            Self::Body
            | Self::Header
            | Self::Footer
            | Self::Footnotes
            | Self::Endnotes
            | Self::Comments
            | Self::Glossary
            | Self::Chart
            | Self::Diagram => PartRole::ElementText,
            Self::Relationships => PartRole::RelationshipTargets,
            Self::Embedding(kind) => PartRole::Binary(kind),
            Self::Metadata | Self::Other => PartRole::Structure,
        }
    }

    /// Whether this part carries redactable *element* text (body, header,
    /// footer, notes, comments, glossary, and the text of charts and diagrams) —
    /// the text/comment/CDATA event model.
    ///
    /// This is narrower than [`is_redactable`](PartKind::is_redactable):
    /// [`Relationships`](PartKind::Relationships) parts are redactable but hold
    /// their PII in attribute values, not element text.
    pub fn is_text(self) -> bool {
        matches!(
            self,
            Self::Body
                | Self::Header
                | Self::Footer
                | Self::Footnotes
                | Self::Endnotes
                | Self::Comments
                | Self::Glossary
                | Self::Chart
                | Self::Diagram
        )
    }

    /// Whether this part yields redactable text of any kind: the element text of
    /// an [`is_text`](PartKind::is_text) part, or the external hyperlink targets
    /// of a [`Relationships`](PartKind::Relationships) part.
    pub fn is_redactable(self) -> bool {
        self.is_text() || matches!(self, Self::Relationships)
    }

    /// The [`EmbeddingKind`] when this part is an embedding.
    pub fn embedding(self) -> Option<EmbeddingKind> {
        match self {
            Self::Embedding(kind) => Some(kind),
            _ => None,
        }
    }

    /// Whether `path` is `{prefix}{n}{suffix}` for some run of digits `n`
    /// (e.g. `word/header2.xml` for prefix `word/header`, suffix `.xml`).
    fn is_numbered(path: &str, prefix: &str, suffix: &str) -> bool {
        let Some(rest) = path.strip_prefix(prefix) else {
            return false;
        };
        let Some(digits) = rest.strip_suffix(suffix) else {
            return false;
        };
        !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
    }
}

/// Classify the binary embedding at `part`, or `None` if it is not one. Word
/// keeps images under `word/media/`, embedded objects under `word/embeddings/`,
/// and fonts under `word/fonts/`.
fn embedding_of(part: &PartPath) -> Option<EmbeddingKind> {
    if part.in_dir("word/media") {
        Some(EmbeddingKind::Image)
    } else if part.in_dir("word/embeddings") {
        Some(EmbeddingKind::Object)
    } else if part.in_dir("word/fonts") {
        Some(EmbeddingKind::Font)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kind of the part at `path`.
    fn kind(path: &str) -> PartKind {
        PartKind::of(&PartPath::from(path))
    }

    #[test]
    fn relationships_parts_are_classified_and_redactable() {
        // The package root rels and a part's sidecar rels are both relationships.
        for path in ["_rels/.rels", "word/_rels/document.xml.rels"] {
            assert_eq!(kind(path), PartKind::Relationships, "{path}");
            assert!(kind(path).is_redactable(), "{path}");
            // They hold attribute text, not element text, so they are redactable
            // but not `is_text`.
            assert!(!kind(path).is_text(), "{path}");
        }
    }

    #[test]
    fn a_non_rels_part_is_not_a_relationships_part() {
        // A path merely containing `_rels` but not ending in `.rels`, and an
        // ordinary XML part, are both left as `Other`.
        assert_eq!(kind("word/settings.xml"), PartKind::Other);
        assert_eq!(kind("word/_rels/document.xml"), PartKind::Other);
    }

    #[test]
    fn is_redactable_covers_every_text_part_and_relationships() {
        assert!(PartKind::Body.is_redactable());
        assert!(PartKind::Relationships.is_redactable());
        // Structure and binary parts are not.
        assert!(!PartKind::Other.is_redactable());
        assert!(!PartKind::Metadata.is_redactable());
        assert!(!PartKind::Embedding(EmbeddingKind::Image).is_redactable());
    }

    #[test]
    fn role_maps_each_kind_to_its_neutral_role() {
        assert_eq!(PartKind::Body.role(), PartRole::ElementText);
        assert_eq!(PartKind::Comments.role(), PartRole::ElementText);
        assert_eq!(
            PartKind::Relationships.role(),
            PartRole::RelationshipTargets
        );
        assert_eq!(
            PartKind::Embedding(EmbeddingKind::Image).role(),
            PartRole::Binary(EmbeddingKind::Image)
        );
        assert_eq!(PartKind::Metadata.role(), PartRole::Structure);
        assert_eq!(PartKind::Other.role(), PartRole::Structure);
    }
}
