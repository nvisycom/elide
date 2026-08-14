//! [`PartKind`] and [`EmbeddingKind`]: the classification of a package part.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// What a package part is, which determines how it is handled.
///
/// The text-bearing WordprocessingML parts ([`Body`](PartKind::Body),
/// [`Header`](PartKind::Header), …) share the `w:t` run text model and are
/// extracted and redacted the same way. [`Embedding`](PartKind::Embedding)
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
    /// A glossary / building-block document (`word/glossary/document.xml`).
    Glossary,
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
        if path == "word/glossary/document.xml" {
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

    /// Whether this part carries redactable `w:t` run text (body, header,
    /// footer, notes, comments, glossary).
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
