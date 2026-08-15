//! [`PartKind`] and [`EmbeddingKind`]: the classification of a package part.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// What a package part is, which determines how it is handled.
///
/// The text-bearing parts ([`Body`](PartKind::Body),
/// [`Header`](PartKind::Header), …) are extracted and redacted the same way: the
/// WordprocessingML stories via the `w:t` run text model, and
/// [`Chart`](PartKind::Chart) / [`Diagram`](PartKind::Diagram) via the
/// user-visible text of their own schemas. [`Embedding`](PartKind::Embedding)
/// parts are binary (images, embedded objects) and are surfaced for redaction
/// as bytes. Everything else is structure or metadata carried through
/// unchanged.
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
    /// A binary embedding (image, embedded object, font).
    Embedding(EmbeddingKind),
    /// Document metadata (`docProps/*`).
    Metadata,
    /// Package structure or any part not otherwise classified (relationships,
    /// `[Content_Types].xml`, settings, styles, …).
    Other,
}

impl PartKind {
    /// Classify the part at `path` from its package path.
    pub(crate) fn of(path: &str) -> Self {
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
        // Binary embeddings.
        if let Some(kind) = EmbeddingKind::of(path) {
            return Self::Embedding(kind);
        }
        // Metadata.
        if path.starts_with("docProps/") {
            return Self::Metadata;
        }
        Self::Other
    }

    /// Whether this part carries redactable text (body, header, footer, notes,
    /// comments, glossary, and the text of charts and diagrams).
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

/// The kind of binary embedding a part holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
#[non_exhaustive]
pub enum EmbeddingKind {
    /// An embedded image (`word/media/*`).
    Image,
    /// An embedded object / OLE package (`word/embeddings/*`).
    Object,
    /// An embedded font (`word/fonts/*`).
    Font,
}

impl EmbeddingKind {
    /// Classify the binary embedding at `path`, or `None` if it is not one.
    fn of(path: &str) -> Option<Self> {
        if path.starts_with("word/media/") {
            Some(Self::Image)
        } else if path.starts_with("word/embeddings/") {
            Some(Self::Object)
        } else if path.starts_with("word/fonts/") {
            Some(Self::Font)
        } else {
            None
        }
    }
}
