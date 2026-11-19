//! What a key the rebuild does not copy held, in the [`HiddenContent`]
//! vocabulary the audit records.

use elide_core::redaction::HiddenContent;
use hayro_syntax::object::Dict;

use crate::pdf::reader::DictExt;

/// What a catalog entry `key` held. The page tree and the attachments are
/// rebuilt rather than copied, so they are never recorded.
pub(super) fn catalog(key: &[u8]) -> Option<HiddenContent> {
    Some(match key {
        b"Type" | b"Pages" | b"Names" | b"AF" | b"Dests" | b"Version" | b"OutputIntents"
        | b"ViewerPreferences" | b"PageLayout" | b"PageMode" | b"Extensions"
        | b"NeedsRendering" => {
            return None;
        }
        b"AcroForm" => HiddenContent::FormField,
        b"Outlines" | b"Threads" | b"PageLabels" => HiddenContent::Outline,
        b"OpenAction" | b"AA" => HiddenContent::Action,
        b"OCProperties" => HiddenContent::Layer,
        b"StructTreeRoot" | b"MarkInfo" => HiddenContent::AlternativeText,
        b"URI" | b"SpiderInfo" => HiddenContent::Link,
        b"DSS" | b"Perms" | b"Legal" => HiddenContent::Signature,
        b"Metadata" | b"Lang" => HiddenContent::Properties,
        _ => HiddenContent::ApplicationData,
    })
}

/// What a catalog name tree `key` held. Named destinations are resolved into
/// the links that use them, and the attachments are rebuilt.
pub(super) fn name_tree(key: &[u8]) -> Option<HiddenContent> {
    Some(match key {
        b"Dests" | b"EmbeddedFiles" => return None,
        b"JavaScript" => HiddenContent::Script,
        b"IDS" | b"URLS" => HiddenContent::Link,
        b"Renditions" | b"AlternatePresentations" => HiddenContent::Control,
        b"AP" => HiddenContent::Annotation,
        b"Pages" | b"Templates" => HiddenContent::FormField,
        _ => HiddenContent::ApplicationData,
    })
}

/// What a page or page-tree entry `key` held. What lays the page out and
/// draws it is rebuilt rather than copied.
pub(super) fn page(key: &[u8]) -> Option<HiddenContent> {
    Some(match key {
        b"Type" | b"Parent" | b"Kids" | b"Count" | b"Resources" | b"MediaBox" | b"CropBox"
        | b"Rotate" | b"UserUnit" | b"Contents" | b"Group" | b"Annots" | b"AF" => return None,
        b"BleedBox" | b"TrimBox" | b"ArtBox" | b"OutputIntents" => return None,
        b"BoxColorInfo" | b"SeparationInfo" => HiddenContent::PrinterSettings,
        b"LastModified"
        | b"Metadata"
        | b"VP"
        | b"DPart"
        | b"Tabs"
        | b"Dur"
        | b"Trans"
        | b"TemplateInstantiated" => HiddenContent::Properties,
        b"Thumb" => HiddenContent::Thumbnail,
        b"AA" | b"PresSteps" => HiddenContent::Action,
        b"B" => HiddenContent::Outline,
        b"ID" | b"PZ" => HiddenContent::Link,
        b"StructParents" => HiddenContent::AlternativeText,
        _ => HiddenContent::ApplicationData,
    })
}

/// What a trailer entry `key` held, or `None` for the file structure a save
/// rewrites.
pub(super) fn trailer(key: &[u8]) -> Option<HiddenContent> {
    match key {
        b"Root" | b"Size" | b"Prev" | b"XRefStm" | b"Type" | b"W" | b"Index" | b"Length"
        | b"Filter" | b"DecodeParms" => None,
        b"Info" => Some(HiddenContent::Properties),
        b"ID" => Some(HiddenContent::DocumentId),
        _ => Some(HiddenContent::ApplicationData),
    }
}

/// What an entry `key` of any other copied object held. A key the
/// specification does not define is application data.
pub(super) fn entry(key: &[u8]) -> Option<HiddenContent> {
    Some(match key {
        b"Metadata" | b"LastModified" | b"Desc" | b"CreationDate" | b"ModDate" | b"CheckSum"
        | b"Mac" | b"CI" | b"RF" | b"Measure" | b"PtData" => HiddenContent::Properties,
        b"PieceInfo" | b"Alternates" => HiddenContent::ApplicationData,
        b"AA" | b"A" => HiddenContent::Action,
        b"Thumb" => HiddenContent::Thumbnail,
        b"OC" => HiddenContent::Layer,
        b"StructParent" | b"StructParents" => HiddenContent::AlternativeText,
        b"OPI" | b"Ref" => HiddenContent::Link,
        _ => HiddenContent::ApplicationData,
    })
}

/// What the annotation `dict` is, by its subtype: a form widget, a link, rich
/// media, or (every other subtype) a markup annotation.
pub(super) fn annotation(dict: &Dict<'_>) -> HiddenContent {
    match dict.name(b"Subtype").as_deref().unwrap_or_default() {
        b"Widget" => HiddenContent::FormField,
        b"Link" => HiddenContent::Link,
        b"Screen" | b"RichMedia" | b"Movie" | b"Sound" | b"3D" => HiddenContent::Control,
        _ => HiddenContent::Annotation,
    }
}
