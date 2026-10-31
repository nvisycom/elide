//! [`HiddenContentPolicy`]: what a codec does with a document's hidden content,
//! and [`HiddenContent`]: the format-neutral vocabulary naming what it strips.

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

/// One item of hidden content a codec strips, named by what it is, never by
/// the format that carries it: EXIF, a PDF's `/Info`, an ID3 tag, and an HTML
/// `<meta>` field are all [`Properties`](Self::Properties).
///
/// Every item belongs to exactly one [`HiddenContentKind`], its
/// [`kind`](Self::kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum HiddenContent {
    /// Descriptive fields about the document: its author, editor, dates,
    /// device, and location, wherever the format keeps them (document
    /// properties, EXIF, XMP, IPTC, audio tags, `<meta>` fields, document
    /// variables).
    Properties,
    /// A custom data store bound into the document, such as custom XML.
    CustomData,
    /// A comment, a reply, or the record of who wrote one.
    Comment,
    /// A tracked insertion, deletion, move, or formatting change, or a shared
    /// workbook's revision log.
    TrackedChange,
    /// A markup annotation drawn over the page: a highlight, a stamp, an ink
    /// drawing, a note.
    Annotation,
    /// A rendered preview of the document or one of its pages.
    Thumbnail,
    /// A reference to a resource outside the document: an attached template, a
    /// linked image or object, another workbook, a URL the document opens.
    Link,
    /// A connection that pulls data into the document from outside: a database
    /// or query connection, a mail-merge data source.
    DataConnection,
    /// A script: JavaScript, an HTML `<script>` or event handler, a
    /// `javascript:` URL.
    Script,
    /// A macro project and the bindings that run it.
    Macro,
    /// An action the document runs when opened or interacted with.
    Action,
    /// An embedded interactive control: an ActiveX control, a plugin object,
    /// rich media.
    Control,
    /// An application add-in the document loads.
    AddIn,
    /// An identifier of an editing session, paragraph, or revision that ties
    /// the document's history together.
    EditingSessionId,
    /// An identifier of the document or file itself.
    DocumentId,
    /// A path on the author's machine.
    FilePath,
    /// The settings of the printer the document was last printed on.
    PrinterSettings,
    /// Data an application keeps for itself.
    ApplicationData,
    /// A digital signature, which names its signer.
    Signature,
    /// A password hash, or the name of a user holding the file.
    Credential,
    /// Content from an earlier revision of the file that is no longer part of
    /// the document.
    PriorRevision,
    /// An interactive form field and its value.
    FormField,
    /// Navigation outside the page: bookmarks, article threads.
    Outline,
    /// Text describing the content for assistive technology, and the
    /// structure that carries it.
    AlternativeText,
    /// An optional-content layer: its configuration, and the content of a
    /// layer hidden by default.
    Layer,
    /// An embedded file no codec can read, so it cannot be redacted.
    Unreadable,
}

impl HiddenContent {
    /// The kind this item belongs to.
    pub const fn kind(self) -> HiddenContentKind {
        match self {
            Self::Properties | Self::CustomData => HiddenContentKind::Metadata,
            Self::Comment | Self::TrackedChange | Self::Annotation => {
                HiddenContentKind::Collaboration
            }
            Self::Thumbnail => HiddenContentKind::Preview,
            Self::Link | Self::DataConnection => HiddenContentKind::ExternalReference,
            Self::Script | Self::Macro | Self::Action | Self::Control | Self::AddIn => {
                HiddenContentKind::ActiveContent
            }
            Self::EditingSessionId
            | Self::DocumentId
            | Self::FilePath
            | Self::PrinterSettings
            | Self::ApplicationData
            | Self::Signature
            | Self::Credential => HiddenContentKind::Fingerprint,
            Self::PriorRevision => HiddenContentKind::Remnant,
            Self::FormField | Self::Outline | Self::AlternativeText | Self::Layer => {
                HiddenContentKind::Unextracted
            }
            Self::Unreadable => HiddenContentKind::EmbeddedObject,
        }
    }
}

/// The kind of a [`HiddenContent`] item. [`Metadata`](Self::Metadata) and
/// [`Collaboration`](Self::Collaboration) are what a [`HiddenContentPolicy`]
/// chooses for; every other kind is always stripped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum HiddenContentKind {
    /// Descriptive data about the document.
    Metadata,
    /// Comments, tracked changes, and annotations.
    Collaboration,
    /// Rendered previews.
    Preview,
    /// References to resources outside the document.
    ExternalReference,
    /// Content that runs: scripts, macros, actions, controls, add-ins.
    ActiveContent,
    /// Traces that identify the author, their machine, or the file's history.
    Fingerprint,
    /// Leftover content that is no longer part of the document.
    Remnant,
    /// Content a reader can reach that no extraction reads, so detection
    /// cannot redact it.
    Unextracted,
    /// Embedded files no codec can read.
    EmbeddedObject,
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
    fn every_item_names_its_kind() {
        assert_eq!(
            HiddenContent::Comment.kind(),
            HiddenContentKind::Collaboration
        );
        assert_eq!(
            HiddenContent::Properties.kind(),
            HiddenContentKind::Metadata
        );
        assert_eq!(
            HiddenContent::Unreadable.kind(),
            HiddenContentKind::EmbeddedObject
        );
    }

    #[test]
    fn builders_set_one_disposition_each() {
        let policy = HiddenContentPolicy::default().with_collaboration(Disposition::Redact);
        assert_eq!(policy.metadata, Disposition::Strip);
        assert_eq!(policy.collaboration, Disposition::Redact);
    }
}
