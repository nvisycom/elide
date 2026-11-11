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
    /// [`KeepOnlyEssential`](Disposition::KeepOnlyEssential) keeps only the
    /// fields the format needs to render correctly (an image's orientation and
    /// colour profile, an HTML page's character set).
    /// [`ScanAsContent`](Disposition::ScanAsContent) keeps the fields the
    /// format labels (a location, an author, a device) and strips the rest;
    /// each labelled field is an entity when the request's catalog declares its
    /// label. [`FlagEverything`](Disposition::FlagEverything) makes every
    /// labelled field an entity.
    pub metadata: Disposition,
    /// What to do with collaboration content.
    ///
    /// [`KeepOnlyEssential`](Disposition::KeepOnlyEssential) produces the final
    /// visible document: tracked insertions are accepted (their text kept),
    /// tracked deletions applied, and comments and annotations dropped with
    /// their authors. [`ScanAsContent`](Disposition::ScanAsContent) keeps them
    /// and detects in their text like body text.
    /// [`FlagEverything`](Disposition::FlagEverything) also makes each
    /// comment, author, and tracked deletion an entity of its own.
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

    /// The disposition this policy sets for hidden content of `kind`, or `None`
    /// for a kind it does not choose for, which is always stripped.
    pub const fn disposition(&self, kind: HiddenContentKind) -> Option<Disposition> {
        match kind {
            HiddenContentKind::Metadata => Some(self.metadata),
            HiddenContentKind::Collaboration => Some(self.collaboration),
            _ => None,
        }
    }
}

/// What happens to one kind of hidden content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Disposition {
    /// Remove the content without inspecting it, keeping only what the format
    /// needs to render the visible document.
    #[default]
    KeepOnlyEssential,
    /// Keep the content and handle it like visible content: what detection
    /// finds under the request's label catalog is redacted, the rest is kept.
    /// Content the format cannot surface to detection is stripped.
    ScanAsContent,
    /// Keep the content as [`ScanAsContent`](Self::ScanAsContent) does, and
    /// make every item the format labels (a metadata field, a comment, an
    /// author, a tracked deletion) an entity, whatever the catalog declares.
    /// The anonymizer decides how each is treated.
    FlagEverything,
}

impl Disposition {
    /// Whether the content is kept and handled as content, as
    /// [`ScanAsContent`](Self::ScanAsContent) and
    /// [`FlagEverything`](Self::FlagEverything) do.
    pub const fn treats_as_content(self) -> bool {
        !matches!(self, Self::KeepOnlyEssential)
    }
}

/// One item of hidden content a codec strips.
///
/// An item is named by what it is, never by the format that carries it: EXIF,
/// a PDF's `/Info`, an ID3 tag, and an HTML `<meta>` field are all
/// [`Properties`](Self::Properties). Every item belongs to exactly one
/// [`HiddenContentKind`], its [`kind`](Self::kind).
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
    /// Content the document carries but never displays: a markup template, an
    /// element marked hidden.
    Undisplayed,
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
            Self::PriorRevision | Self::Undisplayed => HiddenContentKind::Remnant,
            Self::FormField | Self::Outline | Self::AlternativeText | Self::Layer => {
                HiddenContentKind::Unextracted
            }
            Self::Unreadable => HiddenContentKind::EmbeddedObject,
        }
    }
}

/// The kind of a [`HiddenContent`] item.
///
/// [`Metadata`](Self::Metadata) and [`Collaboration`](Self::Collaboration) are
/// what a [`HiddenContentPolicy`] chooses for; every other kind is always
/// stripped.
// Explicit discriminants: `Flagged::hash_into` folds `kind as u8` into the
// tamper-evident hash, so these values are part of the hash. Append new
// variants with the next value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum HiddenContentKind {
    /// Descriptive data about the document.
    Metadata = 0,
    /// Comments, tracked changes, and annotations.
    Collaboration = 1,
    /// Rendered previews.
    Preview = 2,
    /// References to resources outside the document.
    ExternalReference = 3,
    /// Content that runs: scripts, macros, actions, controls, add-ins.
    ActiveContent = 4,
    /// Traces that identify the author, their machine, or the file's history.
    Fingerprint = 5,
    /// Content the file carries that is not part of what a reader sees:
    /// leftovers of earlier revisions, content never displayed.
    Remnant = 6,
    /// Content a reader can reach that no extraction reads, so detection
    /// cannot redact it.
    Unextracted = 7,
    /// Embedded content that cannot be redacted because no codec reads it.
    EmbeddedObject = 8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_strips_all_hidden_content() {
        let policy = HiddenContentPolicy::default();
        assert_eq!(policy.metadata, Disposition::KeepOnlyEssential);
        assert_eq!(policy.collaboration, Disposition::KeepOnlyEssential);
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
        let policy = HiddenContentPolicy::default().with_collaboration(Disposition::ScanAsContent);
        assert_eq!(policy.metadata, Disposition::KeepOnlyEssential);
        assert_eq!(policy.collaboration, Disposition::ScanAsContent);
    }

    #[test]
    fn only_keep_only_essential_strips() {
        assert!(!Disposition::KeepOnlyEssential.treats_as_content());
        assert!(Disposition::ScanAsContent.treats_as_content());
        assert!(Disposition::FlagEverything.treats_as_content());
    }
}
