//! [`PartKind`]: the PresentationML classification of a package part, and its
//! mapping onto the neutral [`PartRole`] the OPC engine acts on.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::opc::{EmbeddingKind, PartPath, PartRole, media_kind};

/// What a PresentationML package part is, which determines how it is handled.
///
/// The text-bearing parts ([`Slide`](PartKind::Slide),
/// [`Notes`](PartKind::Notes), …) carry user-visible text as DrawingML `a:t`
/// runs (or `<t>` in comments), redacted through the shared element-text path.
/// [`Embedding`](PartKind::Embedding) parts are binary (images, media) surfaced
/// as bytes. Everything else is structure or metadata carried through unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
#[non_exhaustive]
pub enum PartKind {
    /// The presentation part (`ppt/presentation.xml`).
    Presentation,
    /// A slide (`ppt/slides/slide{n}.xml`).
    Slide,
    /// A slide layout (`ppt/slideLayouts/slideLayout{n}.xml`).
    SlideLayout,
    /// A slide master (`ppt/slideMasters/slideMaster{n}.xml`).
    SlideMaster,
    /// A notes slide (`ppt/notesSlides/notesSlide{n}.xml`) or the notes master
    /// (`ppt/notesMasters/notesMaster{n}.xml`).
    Notes,
    /// A handout master (`ppt/handoutMasters/handoutMaster{n}.xml`).
    HandoutMaster,
    /// Comment text (`ppt/comments/*.xml` classic, or
    /// `ppt/threadedComments/*.xml` threaded).
    Comments,
    /// Chart text (`ppt/charts/chart{n}.xml`) or a diagram's data
    /// (`ppt/diagrams/data{n}.xml`).
    Chart,
    /// A relationships part (`*/_rels/*.rels`), whose external hyperlink targets
    /// carry the same user data as the slides.
    Relationships,
    /// A binary embedding (image, media, embedded object).
    Embedding(EmbeddingKind),
    /// Document metadata (`docProps/*`).
    Metadata,
    /// Package structure or any part not otherwise classified (the theme,
    /// presentation properties, `[Content_Types].xml`, …).
    Other,
}

impl PartKind {
    /// Classify `part` from its package path.
    pub fn of(part: &PartPath) -> Self {
        let path = part.as_str();
        if path == "ppt/presentation.xml" {
            return Self::Presentation;
        }
        if Self::is_numbered(path, "ppt/slides/slide", ".xml") {
            return Self::Slide;
        }
        if Self::is_numbered(path, "ppt/slideLayouts/slideLayout", ".xml") {
            return Self::SlideLayout;
        }
        if Self::is_numbered(path, "ppt/slideMasters/slideMaster", ".xml") {
            return Self::SlideMaster;
        }
        if Self::is_numbered(path, "ppt/notesSlides/notesSlide", ".xml")
            || Self::is_numbered(path, "ppt/notesMasters/notesMaster", ".xml")
        {
            return Self::Notes;
        }
        if Self::is_numbered(path, "ppt/handoutMasters/handoutMaster", ".xml") {
            return Self::HandoutMaster;
        }
        // Comments come in three layouts, all holding the comment text: the
        // classic per-slide `ppt/comments/`, and the modern threaded
        // `ppt/threadedComments/` (PowerPoint 2021+). `ppt/commentAuthors.xml`
        // is authors-only, so only the comment parts themselves are matched.
        if (path.starts_with("ppt/comments/") || path.starts_with("ppt/threadedComments/"))
            && path.ends_with(".xml")
        {
            return Self::Comments;
        }
        if Self::is_numbered(path, "ppt/charts/chart", ".xml")
            || Self::is_numbered(path, "ppt/diagrams/data", ".xml")
        {
            return Self::Chart;
        }
        if part.is_relationships() {
            return Self::Relationships;
        }
        if let Some(kind) = embedding_kind(part) {
            return Self::Embedding(kind);
        }
        if path.starts_with("docProps/") {
            return Self::Metadata;
        }
        Self::Other
    }

    /// The neutral [`PartRole`] the OPC engine applies to this kind: text-bearing
    /// parts extract element text, relationships parts their hyperlink targets,
    /// embeddings surface as bytes, and the rest pass through as structure.
    pub(crate) fn role(self) -> PartRole {
        match self {
            Self::Slide
            | Self::SlideLayout
            | Self::SlideMaster
            | Self::Notes
            | Self::HandoutMaster
            | Self::Comments
            | Self::Chart => PartRole::ElementText,
            Self::Relationships => PartRole::RelationshipTargets,
            Self::Embedding(kind) => PartRole::Binary(kind),
            Self::Presentation | Self::Metadata | Self::Other => PartRole::Structure,
        }
    }

    /// The [`EmbeddingKind`] when this part is a binary embedding.
    pub fn embedding(self) -> Option<EmbeddingKind> {
        match self {
            Self::Embedding(kind) => Some(kind),
            _ => None,
        }
    }

    /// Whether `path` is `{prefix}{n}{suffix}` for some run of digits `n`.
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

/// The [`EmbeddingKind`] of a binary media part, if `part` names one.
///
/// `ppt/media/` mixes images, audio, and video, so the kind is taken from the
/// file extension (a slide's embedded `.mp3` is [`Audio`], not an image).
///
/// [`Audio`]: EmbeddingKind::Audio
fn embedding_kind(part: &PartPath) -> Option<EmbeddingKind> {
    if part.in_dir("ppt/media") {
        Some(media_kind(part.as_str()))
    } else if part.in_dir("ppt/embeddings") {
        Some(EmbeddingKind::Object)
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
    fn classifies_presentationml_parts() {
        assert_eq!(kind("ppt/presentation.xml"), PartKind::Presentation);
        assert_eq!(kind("ppt/slides/slide1.xml"), PartKind::Slide);
        assert_eq!(
            kind("ppt/slideLayouts/slideLayout2.xml"),
            PartKind::SlideLayout
        );
        assert_eq!(kind("ppt/notesSlides/notesSlide1.xml"), PartKind::Notes);
        assert_eq!(kind("ppt/comments/comment1.xml"), PartKind::Comments);
        // Threaded comments (PowerPoint 2021+) carry comment text too, so they
        // must classify as text-bearing — not fall through to `Other` and leak.
        assert_eq!(
            kind("ppt/threadedComments/threadedComment1.xml"),
            PartKind::Comments
        );
        assert_eq!(
            kind("ppt/slides/_rels/slide1.xml.rels"),
            PartKind::Relationships
        );
        assert_eq!(
            kind("ppt/media/image1.png"),
            PartKind::Embedding(EmbeddingKind::Image)
        );
        // A slide's `ppt/media/` dir also holds audio and video, classified by
        // extension so an embedded clip surfaces as media, not as an image.
        assert_eq!(
            kind("ppt/media/media1.mp3"),
            PartKind::Embedding(EmbeddingKind::Audio)
        );
        assert_eq!(
            kind("ppt/media/media2.mp4"),
            PartKind::Embedding(EmbeddingKind::Video)
        );
        assert_eq!(kind("ppt/theme/theme1.xml"), PartKind::Other);
    }

    #[test]
    fn roles_map_text_parts_to_element_text() {
        assert_eq!(PartKind::Slide.role(), PartRole::ElementText);
        assert_eq!(PartKind::Notes.role(), PartRole::ElementText);
        assert_eq!(
            PartKind::Relationships.role(),
            PartRole::RelationshipTargets
        );
        assert_eq!(PartKind::Presentation.role(), PartRole::Structure);
        assert_eq!(
            PartKind::Embedding(EmbeddingKind::Image).role(),
            PartRole::Binary(EmbeddingKind::Image)
        );
    }
}
