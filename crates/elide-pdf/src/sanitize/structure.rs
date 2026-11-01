//! The structure a sanitized PDF keeps: the catalog, each page and page-tree
//! node, and the trailer keep an allowlist of entries; every dictionary loses
//! the entries that hold hidden content wherever they are; and the catalog's
//! name trees are reduced to navigation and attachments.

use std::collections::BTreeSet;

use elide_codec::StrippedContent;
use elide_core::redaction::HiddenContent;
use lopdf::{Dictionary, Document, Object, ObjectId};

use super::ext::SanitizeDictionaryExt;
use crate::lopdf_ext::{DictionaryExt, ObjectExt};

/// The catalog entries kept: the page tree, the kept name trees and named
/// destinations, how the document opens and reads, and its associated files
/// (surfaced as content). Every other entry is removed.
const CATALOG_KEPT: [&[u8]; 11] = [
    b"Type",
    b"Version",
    b"Extensions",
    b"Pages",
    b"Names",
    b"Dests",
    b"ViewerPreferences",
    b"PageLayout",
    b"PageMode",
    b"Lang",
    b"AF",
];

/// The page entries kept: what lays the page out and draws it, its kept
/// annotations and associated files, and its presentation timing. Every other
/// entry is removed.
const PAGE_KEPT: [&[u8]; 19] = [
    b"Type",
    b"Parent",
    b"Resources",
    b"MediaBox",
    b"CropBox",
    b"BleedBox",
    b"TrimBox",
    b"ArtBox",
    b"BoxColorInfo",
    b"Contents",
    b"Rotate",
    b"Group",
    b"UserUnit",
    b"Tabs",
    b"Annots",
    b"AF",
    b"Trans",
    b"Dur",
    b"VP",
];

/// The page-tree node entries kept: the tree itself and the page attributes a
/// node passes down.
const PAGE_TREE_KEPT: [&[u8]; 8] = [
    b"Type",
    b"Parent",
    b"Kids",
    b"Count",
    b"Resources",
    b"MediaBox",
    b"CropBox",
    b"Rotate",
];

/// The trailer entries kept: the catalog and the object count. The save
/// rewrites the cross-reference entries; every other entry is removed.
const TRAILER_KEPT: [&[u8]; 2] = [b"Root", b"Size"];

/// The name-tree entries kept in the catalog's `/Names`: named destinations
/// (navigation) and the attachments, which are surfaced as content.
const NAMES_KEPT: [&[u8]; 2] = [b"Dests", b"EmbeddedFiles"];

/// The keys removed from every dictionary, each with the hidden content it
/// holds.
const EVERYWHERE_REMOVED: [(&[u8], HiddenContent); 5] = [
    (b"PieceInfo", HiddenContent::ApplicationData),
    (b"OC", HiddenContent::Layer),
    (b"AA", HiddenContent::Action),
    (b"Thumb", HiddenContent::Thumbnail),
    (b"Metadata", HiddenContent::Properties),
];

/// Remove every catalog entry but the kept ones and every name tree but the
/// kept ones, recording each in `stripped`.
pub(super) fn reduce_catalog(doc: &mut Document, stripped: &mut StrippedContent) {
    let names = doc
        .catalog()
        .ok()
        .and_then(|catalog| catalog.get(b"Names").ok())
        .cloned();
    if let Some(Object::Reference(id)) = names
        && let Some(names) = doc
            .get_object_mut(id)
            .ok()
            .and_then(|o| o.as_dict_mut().ok())
    {
        retain_kept_names(names, stripped);
    }
    let Ok(catalog) = doc.catalog_mut() else {
        return;
    };
    if let Ok(Object::Dictionary(names)) = catalog.get_mut(b"Names") {
        retain_kept_names(names, stripped);
    }
    for (_, content) in catalog.retain_keys(&CATALOG_KEPT, catalog_content) {
        stripped.record(content, None, 1);
    }
}

/// What a removed catalog entry `key` held.
fn catalog_content(key: &[u8]) -> Option<HiddenContent> {
    Some(match key {
        b"AcroForm" => HiddenContent::FormField,
        b"Outlines" | b"Threads" | b"PageLabels" => HiddenContent::Outline,
        b"OpenAction" | b"AA" => HiddenContent::Action,
        b"OCProperties" => HiddenContent::Layer,
        b"StructTreeRoot" | b"MarkInfo" => HiddenContent::AlternativeText,
        b"URI" | b"SpiderInfo" => HiddenContent::Link,
        b"DSS" | b"Perms" | b"Legal" => HiddenContent::Signature,
        b"Metadata" => HiddenContent::Properties,
        _ => HiddenContent::ApplicationData,
    })
}

/// What a removed page or page-tree entry `key` held.
fn page_content(key: &[u8]) -> Option<HiddenContent> {
    Some(match key {
        b"LastModified" | b"Metadata" => HiddenContent::Properties,
        b"Thumb" => HiddenContent::Thumbnail,
        b"AA" => HiddenContent::Action,
        b"B" => HiddenContent::Outline,
        b"ID" | b"PZ" => HiddenContent::Link,
        b"StructParents" => HiddenContent::AlternativeText,
        _ => HiddenContent::ApplicationData,
    })
}

/// What a removed trailer entry `key` held, or `None` for the cross-reference
/// entries the save rewrites (including those a cross-reference stream's
/// dictionary brings into the trailer), which are file structure rather than
/// content.
fn trailer_content(key: &[u8]) -> Option<HiddenContent> {
    match key {
        b"Prev" | b"XRefStm" | b"Type" | b"W" | b"Index" | b"Length" | b"Filter"
        | b"DecodeParms" => None,
        b"Info" => Some(HiddenContent::Properties),
        b"ID" => Some(HiddenContent::DocumentId),
        _ => Some(HiddenContent::ApplicationData),
    }
}

/// Record the property lists of a removed resource `/Properties` entry: an
/// optional-content group (one of `groups`, or written inline) as layer
/// configuration, any other as alternate text.
fn record_property_lists(
    lists: &Object,
    groups: &BTreeSet<ObjectId>,
    stripped: &mut StrippedContent,
) {
    let Object::Dictionary(lists) = lists else {
        stripped.record(HiddenContent::Layer, None, 1);
        return;
    };
    for (_, list) in lists.iter() {
        let group = match list {
            Object::Reference(id) => groups.contains(id),
            Object::Dictionary(dict) => dict.is_content_group(),
            _ => false,
        };
        let content = if group {
            HiddenContent::Layer
        } else {
            HiddenContent::AlternativeText
        };
        stripped.record(content, None, 1);
    }
}

/// Drop every entry of a `/Names` dictionary but the kept name trees,
/// recording each by the hidden content it holds.
fn retain_kept_names(names: &mut Dictionary, stripped: &mut StrippedContent) {
    for (_, content) in names.retain_keys(&NAMES_KEPT, |key| Some(name_tree_content(key))) {
        stripped.record(content, None, 1);
    }
}

/// The hidden content a catalog name tree holds, by its key.
fn name_tree_content(key: &[u8]) -> HiddenContent {
    match key {
        b"JavaScript" => HiddenContent::Script,
        b"IDS" | b"URLS" => HiddenContent::Link,
        b"Renditions" | b"AlternatePresentations" => HiddenContent::Control,
        b"AP" => HiddenContent::Annotation,
        b"Pages" | b"Templates" => HiddenContent::FormField,
        _ => HiddenContent::ApplicationData,
    }
}

/// Reduce every page dictionary and page-tree node to what lays the pages out
/// and draws them, recording each removed entry.
pub(super) fn reduce_pages(doc: &mut Document, stripped: &mut StrippedContent) {
    for object in doc.objects.values_mut() {
        let Some(dict) = object.dict_mut() else {
            continue;
        };
        let kept: &[&[u8]] = if dict.has_type(b"Page") {
            &PAGE_KEPT
        } else if dict.has_type(b"Pages") {
            &PAGE_TREE_KEPT
        } else {
            continue;
        };
        for (_, content) in dict.retain_keys(kept, page_content) {
            stripped.record(content, None, 1);
        }
    }
}

/// Remove, from every dictionary, the entries that hold hidden content
/// wherever they are, an attachment's description and timestamps, and the
/// property lists a resource dictionary names, recording each.
pub(super) fn strip_everywhere(doc: &mut Document, stripped: &mut StrippedContent) {
    // An optional-content group a property list names was the layer
    // configuration; anything else it names is marked content's alternate
    // text, which nothing extracts.
    let groups: BTreeSet<ObjectId> = doc
        .objects
        .iter()
        .filter(|(_, object)| object.dict().is_some_and(Dictionary::is_content_group))
        .map(|(&id, _)| id)
        .collect();

    // A resource dictionary's `/Properties` names the optional-content groups
    // its content marks; with the groups gone, the names go too. A resource
    // dictionary is inline or referenced, so find the referenced ones first.
    let resources: BTreeSet<ObjectId> = doc
        .objects
        .values()
        .filter_map(ObjectExt::dict)
        .filter_map(|dict| dict.get(b"Resources").and_then(Object::as_reference).ok())
        .collect();
    for (id, object) in &mut doc.objects {
        let Some(dict) = object.dict_mut() else {
            continue;
        };
        for (key, content) in EVERYWHERE_REMOVED {
            stripped.record(content, None, dict.remove_counted(key));
        }
        stripped.record(
            HiddenContent::Properties,
            None,
            dict.strip_attachment_metadata(),
        );
        if resources.contains(id)
            && let Some(lists) = dict.remove(b"Properties")
        {
            record_property_lists(&lists, &groups, stripped);
        }
        if let Ok(Object::Dictionary(inline)) = dict.get_mut(b"Resources")
            && let Some(lists) = inline.remove(b"Properties")
        {
            record_property_lists(&lists, &groups, stripped);
        }
    }
}

/// Reduce the trailer to the catalog and the object count, recording each
/// removed entry.
pub(super) fn reduce_trailer(doc: &mut Document, stripped: &mut StrippedContent) {
    for (_, content) in doc.trailer.retain_keys(&TRAILER_KEPT, trailer_content) {
        stripped.record(content, None, 1);
    }
}
