//! [`HiddenContentPolicy`]: what a codec does with a document's hidden content.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// What a codec does with the content a reader of the document never sees.
///
/// A document carries two kinds of content. Visible content (body text,
/// pixels, cells, speech) is always kept and redacted through detection.
/// Hidden content is handled at decode, before detection runs, so detection
/// sees exactly what will ship. Only two kinds of hidden content are a choice,
/// because only for these do reasonable callers want different outcomes:
///
/// - [`metadata`](Self::metadata): document properties, EXIF and XMP, audio
///   tags, HTML `<meta>` fields;
/// - [`collaboration`](Self::collaboration): comments, tracked revisions,
///   annotations.
///
/// Everything else hidden is handled the same way whatever the policy:
/// previews and thumbnails, external references, active content (scripts,
/// macros, add-ins), and editor fingerprints are always stripped; an embedded
/// object is decoded and redacted as content when a codec can read it and
/// stripped when none can; and structure a codec does not recognize rejects
/// the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct HiddenContentPolicy {
    /// What to do with document metadata.
    ///
    /// [`Strip`](Disposition::Strip) keeps only the fields the format needs to
    /// render correctly (an image's orientation and colour profile, an HTML
    /// page's character set).
    pub metadata: Disposition,
    /// What to do with collaboration content.
    ///
    /// [`Strip`](Disposition::Strip) produces the final visible document:
    /// tracked insertions are accepted (their text kept), tracked deletions
    /// applied, and comments and annotations dropped with their authors.
    pub collaboration: Disposition,
}

impl HiddenContentPolicy {
    /// This policy with its metadata disposition set to `metadata`.
    #[must_use]
    pub const fn with_metadata(mut self, metadata: Disposition) -> Self {
        self.metadata = metadata;
        self
    }

    /// This policy with its collaboration disposition set to `collaboration`.
    #[must_use]
    pub const fn with_collaboration(mut self, collaboration: Disposition) -> Self {
        self.collaboration = collaboration;
        self
    }
}

/// What happens to one kind of hidden content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Disposition {
    /// Strip the content down to what a reader of the document sees.
    #[default]
    Strip,
    /// Keep the content and redact it through detection like any other
    /// content, its author and date fields included. A format that cannot
    /// surface the content to detection strips it instead.
    Redact,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_strips_all_hidden_content() {
        let policy = HiddenContentPolicy::default();
        assert_eq!(policy.metadata, Disposition::Strip);
        assert_eq!(policy.collaboration, Disposition::Strip);
    }

    #[test]
    fn builders_set_one_disposition_each() {
        let policy = HiddenContentPolicy::default().with_collaboration(Disposition::Redact);
        assert_eq!(policy.metadata, Disposition::Strip);
        assert_eq!(policy.collaboration, Disposition::Redact);
    }
}
