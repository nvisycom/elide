//! Remove a PDF's hidden content before its text is extracted.
//!
//! A PDF carries content outside its page content streams that a reader of the
//! page never sees, or that detection never reads. The sanitizer removes it at
//! decode, so detection runs over exactly what will ship, and records each item
//! it removes by the [`HiddenContent`] vocabulary:
//!
//! - **Metadata**: the trailer `/Info` dictionary, every `/Metadata` XMP
//!   stream (the catalog's, a page's, an image's), and an attachment's
//!   description and timestamps.
//! - **Annotations**, every one but an internal link, each by what it is: a
//!   markup annotation, a form widget, a link that leaves the document or
//!   chains a further action, or rich media. A kept link keeps only where it is
//!   and where it goes, not its tooltip, author, dates, or name.
//! - **Active content**: document and page actions (`/OpenAction`, `/AA`), and
//!   the JavaScript and rendition name trees.
//! - **Content no extraction reads**: interactive forms (`/AcroForm`), the
//!   outline, article threads, and page labels, the logical structure tree,
//!   and the alternate text marked content carries in the content streams.
//! - **Optional-content layers**: the content of every layer hidden by default
//!   is cut from the pages (see [`content`](crate::content)), then the layer
//!   configuration goes, so what remains shows as it did.
//! - **Previews and fingerprints**: page thumbnails, application private data
//!   (`/PieceInfo`), signature data (`/DSS`, `/Perms`), and the trailer `/ID`.
//! - **Everything else** outside what draws the pages: the catalog, each page
//!   and page-tree node, and the trailer keep an allowlist of entries, and
//!   every other entry goes.
//! - **Prior revisions**: every object no longer reachable from the document,
//!   a previous revision's content.
//!
//! Embedded files are not hidden content: the attachment tree and the `/AF`
//! associations are kept, and each file is surfaced for its own codec to
//! redact (see [`attachment`](crate::attachment)). Their names, which a viewer
//! shows but detection never sees, are made neutral.
//!
//! A PDF surfaces neither its metadata nor its annotations to detection, so
//! both are stripped whatever the
//! [`HiddenContentPolicy`](elide_core::redaction::HiddenContentPolicy) asks: a
//! format that cannot redact hidden content strips it.
//!
//! Each removal drops the reference; a final prune deletes every object no
//! surviving object reaches, so a removed structure's whole subtree goes without
//! touching anything still in use.

use std::collections::{BTreeSet, HashSet};

use elide_codec::StrippedContent;
use elide_core::redaction::HiddenContent;
use elide_core::{Error, ErrorKind, Result};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

use crate::attachment::neutralize_names;
use crate::content::{remove_images, sanitize_content};
use crate::document::{Store, is_xref_stream, save};
use crate::extract::EmbeddingKind;
use crate::samples;

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

/// The entries a kept internal link keeps: where it is, where it goes, and how
/// it looks. Its tooltip, author, dates, and name go.
const LINK_KEPT: [&[u8]; 12] = [
    b"Type",
    b"Subtype",
    b"Rect",
    b"Dest",
    b"A",
    b"Border",
    b"BS",
    b"C",
    b"H",
    b"QuadPoints",
    b"F",
    b"P",
];

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

/// The document in `store` with its hidden content removed, as new bytes with
/// the record of what was removed, or `None` when it carries none.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if the sanitized document cannot be saved.
pub(crate) fn sanitize_store(store: &Store) -> Result<Option<(Vec<u8>, StrippedContent)>> {
    let mut doc = store.clone_doc();
    let stripped = sanitize(&mut doc)?;
    if stripped.is_empty() {
        return Ok(None);
    }
    Ok(Some((save(&mut doc)?, stripped)))
}

/// Check that the document in `store` carries no hidden content.
///
/// # Errors
///
/// [`ErrorKind::Sanitization`] naming what survived.
pub(crate) fn check_store(store: &Store) -> Result<()> {
    let survived = sanitize(&mut store.clone_doc())?;
    if survived.is_empty() {
        return Ok(());
    }
    let items: Vec<String> = survived
        .iter()
        .map(|entry| format!("{:?}", entry.content))
        .collect();
    Err(Error::new(
        ErrorKind::Sanitization,
        format!(
            "encoded PDF still carries hidden content: {}",
            items.join(", ")
        ),
    ))
}

/// Remove `doc`'s hidden content, returning the record of what was removed.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if a content stream that may hold a hidden
/// layer's content cannot be decoded or encoded.
pub(crate) fn sanitize(doc: &mut Document) -> Result<StrippedContent> {
    let mut stripped = StrippedContent::new();

    // A cross-reference stream is file structure the next save rewrites, not
    // content: a loaded one is a stale copy, dropped before anything is
    // counted. What is unreachable now is a prior revision's content; what the
    // removals below leave unreachable is counted with the removal itself.
    doc.objects.retain(|_, object| !is_xref_stream(object));
    stripped.record(
        HiddenContent::PriorRevision,
        None,
        doc.prune_objects().len(),
    );

    // A hidden layer's content goes before the layer configuration that says
    // it is hidden.
    let cuts = sanitize_content(doc)?;
    stripped.record(HiddenContent::Layer, None, cuts.layers);
    stripped.record(HiddenContent::AlternativeText, None, cuts.alternate_text);

    // An image that cannot be surfaced for detection (fax or JBIG2 samples,
    // raw samples of a kind the decoder does not read, a stencil mask), and
    // every inline image, is removed rather than shipped unscanned.
    let unsurfaced: HashSet<ObjectId> = doc
        .objects
        .iter()
        .filter_map(|(&id, object)| {
            let Object::Stream(stream) = object else {
                return None;
            };
            let image = stream
                .dict
                .get(b"Subtype")
                .and_then(Object::as_name)
                .is_ok_and(|subtype| subtype == b"Image");
            (image && !is_surfaceable(doc, stream)).then_some(id)
        })
        .collect();
    stripped.record(
        HiddenContent::Unreadable,
        None,
        remove_images(doc, &unsurfaced, true)?,
    );
    // What only a hidden layer used is gone with it, counted with the cut.
    doc.prune_objects();

    for page in doc.get_pages().into_values() {
        for content in keep_internal_links(doc, page) {
            stripped.record(content, None, 1);
        }
    }
    sanitize_catalog(doc, &mut stripped);
    stripped.record(HiddenContent::Properties, None, neutralize_names(doc)?);

    // The page dictionaries and page-tree nodes keep only what lays the pages
    // out and draws them.
    for object in doc.objects.values_mut() {
        let Some(dict) = dictionary_mut(object) else {
            continue;
        };
        let kept: &[&[u8]] = if dict.has_type(b"Page") {
            &PAGE_KEPT
        } else if dict.has_type(b"Pages") {
            &PAGE_TREE_KEPT
        } else {
            continue;
        };
        for (_, content) in retain_keys(dict, kept, page_content) {
            stripped.record(content, None, 1);
        }
    }

    // An optional-content group a property list names was the layer
    // configuration; anything else it names is marked content's alternate
    // text, which nothing extracts.
    let groups: BTreeSet<ObjectId> = doc
        .objects
        .iter()
        .filter(|(_, object)| dictionary(object).is_some_and(is_content_group))
        .map(|(&id, _)| id)
        .collect();

    // A resource dictionary's `/Properties` names the optional-content groups
    // its content marks; with the groups gone, the names go too. A resource
    // dictionary is inline or referenced, so find the referenced ones first.
    let resources: BTreeSet<ObjectId> = doc
        .objects
        .values()
        .filter_map(dictionary)
        .filter_map(|dict| dict.get(b"Resources").and_then(Object::as_reference).ok())
        .collect();
    for (id, object) in &mut doc.objects {
        let Some(dict) = dictionary_mut(object) else {
            continue;
        };
        for (key, content) in EVERYWHERE_REMOVED {
            stripped.record(content, None, removed(dict, key));
        }
        stripped.record(
            HiddenContent::Properties,
            None,
            sanitize_attachment_metadata(dict),
        );
        if resources.contains(id)
            && let Some(lists) = dict.remove(b"Properties")
        {
            record_property_lists(&lists, &groups, &mut stripped);
        }
        if let Ok(Object::Dictionary(inline)) = dict.get_mut(b"Resources")
            && let Some(lists) = inline.remove(b"Properties")
        {
            record_property_lists(&lists, &groups, &mut stripped);
        }
    }

    for (_, content) in retain_keys(&mut doc.trailer, &TRAILER_KEPT, trailer_content) {
        stripped.record(content, None, 1);
    }
    doc.prune_objects();
    Ok(stripped)
}

/// Remove every annotation of `page` but its internal links, returning the
/// hidden content each removed one was.
fn keep_internal_links(doc: &mut Document, page: ObjectId) -> Vec<HiddenContent> {
    let annotations: Vec<Object> = match doc.get_dictionary(page).and_then(|p| p.get(b"Annots")) {
        Ok(Object::Reference(id)) => match doc.get_object(*id).and_then(Object::as_array) {
            Ok(array) => array.clone(),
            Err(_) => Vec::new(),
        },
        Ok(Object::Array(array)) => array.clone(),
        _ => return Vec::new(),
    };
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    for annotation in annotations {
        let dict = match &annotation {
            Object::Reference(id) => doc.get_dictionary(*id).ok(),
            Object::Dictionary(dict) => Some(dict),
            _ => None,
        };
        match dict {
            Some(dict) if is_internal_link(dict) => kept.push(annotation),
            Some(dict) => dropped.push(annotation_content(dict)),
            None => dropped.push(HiddenContent::Annotation),
        }
    }
    // A kept link keeps only where it is and where it goes: its tooltip,
    // author, dates, and name are an annotation's text.
    let mut reduced = false;
    for annotation in &mut kept {
        let dict = match annotation {
            Object::Reference(id) => doc
                .get_object_mut(*id)
                .ok()
                .and_then(|o| o.as_dict_mut().ok()),
            Object::Dictionary(dict) => {
                reduced = true;
                Some(dict)
            }
            _ => None,
        };
        if let Some(dict) = dict
            && !retain_keys(dict, &LINK_KEPT, |_| Some(HiddenContent::Annotation)).is_empty()
        {
            dropped.push(HiddenContent::Annotation);
        }
    }
    if dropped.is_empty() && !reduced {
        return dropped;
    }
    if let Some(page) = doc
        .get_object_mut(page)
        .ok()
        .and_then(|o| o.as_dict_mut().ok())
    {
        if kept.is_empty() {
            page.remove(b"Annots");
        } else {
            page.set("Annots", Object::Array(kept));
        }
    }
    dropped
}

/// The hidden content an annotation is, by its subtype: a form widget, a link,
/// rich media, or (every other subtype) a markup annotation.
fn annotation_content(annotation: &Dictionary) -> HiddenContent {
    let subtype = annotation.get(b"Subtype").and_then(Object::as_name);
    match subtype.unwrap_or_default() {
        b"Widget" => HiddenContent::FormField,
        b"Link" => HiddenContent::Link,
        b"Screen" | b"RichMedia" | b"Movie" | b"Sound" | b"3D" => HiddenContent::Control,
        _ => HiddenContent::Annotation,
    }
}

/// Whether an annotation is a link to a place in this same document: a `/Link`
/// with a `/Dest`, or whose action is an internal `/GoTo` that chains no
/// further action (a `/Next` can run script after the jump).
fn is_internal_link(annotation: &Dictionary) -> bool {
    let is_link = annotation
        .get(b"Subtype")
        .and_then(Object::as_name)
        .is_ok_and(|subtype| subtype == b"Link");
    if !is_link {
        return false;
    }
    match annotation.get(b"A").and_then(Object::as_dict) {
        Ok(action) => {
            let go_to = action
                .get(b"S")
                .and_then(Object::as_name)
                .is_ok_and(|kind| kind == b"GoTo");
            go_to && action.get(b"Next").is_err()
        }
        Err(_) => annotation.get(b"Dest").is_ok(),
    }
}

/// Remove every catalog entry but the kept ones and every name tree but the
/// kept ones, recording each in `stripped`.
fn sanitize_catalog(doc: &mut Document, stripped: &mut StrippedContent) {
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
    for (_, content) in retain_keys(catalog, &CATALOG_KEPT, catalog_content) {
        stripped.record(content, None, 1);
    }
}

/// Whether the image XObject `stream` can be surfaced to the image pipeline: a
/// JPEG or JPEG 2000 file as it is, or raw samples the decoder reads (as a
/// PNG).
fn is_surfaceable(doc: &Document, stream: &Stream) -> bool {
    match image_kind(stream) {
        EmbeddingKind::Jpeg | EmbeddingKind::Jpeg2000 => true,
        EmbeddingKind::Raw => samples::is_supported(doc, &stream.dict),
        EmbeddingKind::CcittFax | EmbeddingKind::Jbig2 => false,
    }
}

/// The encoding of the image XObject `stream`, by its filters.
fn image_kind(stream: &Stream) -> EmbeddingKind {
    let filters: Option<Vec<String>> = stream.filters().ok().map(|filters| {
        filters
            .iter()
            .map(|name| String::from_utf8_lossy(name).into_owned())
            .collect()
    });
    EmbeddingKind::from_filters(filters.as_deref())
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

/// Remove every entry of `dict` but the `kept` ones, returning each removed
/// key with what `content` says it held (an entry it says nothing for goes
/// unrecorded).
fn retain_keys(
    dict: &mut Dictionary,
    kept: &[&[u8]],
    content: impl Fn(&[u8]) -> Option<HiddenContent>,
) -> Vec<(Vec<u8>, HiddenContent)> {
    let removed: Vec<Vec<u8>> = dict
        .iter()
        .map(|(key, _)| key.clone())
        .filter(|key| !kept.contains(&key.as_slice()))
        .collect();
    removed
        .into_iter()
        .filter_map(|key| {
            dict.remove(&key);
            let held = content(&key)?;
            Some((key, held))
        })
        .collect()
}

/// Whether `dict` is an optional-content group or membership dictionary.
fn is_content_group(dict: &Dictionary) -> bool {
    dict.has_type(b"OCG") || dict.has_type(b"OCMD")
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
            Object::Dictionary(dict) => is_content_group(dict),
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
    let dropped: Vec<Vec<u8>> = names
        .iter()
        .map(|(key, _)| key.clone())
        .filter(|key| !NAMES_KEPT.contains(&key.as_slice()))
        .collect();
    for key in dropped {
        names.remove(&key);
        stripped.record(name_tree_content(&key), None, 1);
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

/// Remove an attachment file specification's description and its embedded
/// file's timestamps and checksum, returning how many were removed.
fn sanitize_attachment_metadata(dict: &mut Dictionary) -> usize {
    let mut count = 0;
    if dict.has_type(b"Filespec") {
        count += removed(dict, b"Desc");
    }
    if dict.has_type(b"EmbeddedFile")
        && let Ok(Object::Dictionary(params)) = dict.get_mut(b"Params")
    {
        for key in [&b"CreationDate"[..], b"ModDate", b"CheckSum", b"Mac"] {
            count += removed(params, key);
        }
    }
    count
}

/// `object`'s dictionary: its own, or a stream's.
fn dictionary(object: &Object) -> Option<&Dictionary> {
    match object {
        Object::Dictionary(dict) => Some(dict),
        Object::Stream(stream) => Some(&stream.dict),
        _ => None,
    }
}

/// `object`'s dictionary, mutably: its own, or a stream's.
fn dictionary_mut(object: &mut Object) -> Option<&mut Dictionary> {
    match object {
        Object::Dictionary(dict) => Some(dict),
        Object::Stream(stream) => Some(&mut stream.dict),
        _ => None,
    }
}

/// Remove `key` from `dict`, returning 1 when it was present and 0 otherwise.
fn removed(dict: &mut Dictionary, key: &[u8]) -> usize {
    usize::from(dict.remove(key).is_some())
}

#[cfg(test)]
mod tests {
    use lopdf::content::{Content, Operation};
    use lopdf::{Stream, StringFormat, dictionary};

    use super::*;

    /// A one-page document whose page carries `annotations` and whose catalog
    /// and trailer carry every hidden structure the sanitizer removes.
    fn document(annotations: Vec<Object>) -> Document {
        let mut doc = Document::with_version("1.7");
        let content = Content {
            operations: vec![Operation::new("BT", vec![]), Operation::new("ET", vec![])],
        };
        let contents = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let pages = doc.new_object_id();
        let xmp = doc.add_object(Stream::new(
            dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
            b"<x:xmpmeta>Jane Roe</x:xmpmeta>".to_vec(),
        ));
        let thumb = doc.add_object(Stream::new(dictionary! {}, b"thumbnail".to_vec()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages,
            "Contents" => contents,
            "Annots" => annotations,
            "Thumb" => thumb,
            "PieceInfo" => dictionary! { "Illustrator" => "private" },
        });
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page.into()],
                "Count" => 1,
            }),
        );
        let javascript =
            doc.add_object(dictionary! { "S" => "JavaScript", "JS" => "app.alert(1)" });
        let catalog = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages,
            "Metadata" => xmp,
            "OpenAction" => javascript,
            "Outlines" => dictionary! { "Title" => Object::string_literal("Jane's notes") },
            "Names" => dictionary! {
                "JavaScript" => dictionary! { "Names" => vec![Object::string_literal("a"), javascript.into()] },
                "Dests" => dictionary! {},
            },
        });
        let info = doc.add_object(dictionary! {
            "Author" => Object::String(b"Jane Roe".to_vec(), StringFormat::Literal),
        });
        doc.trailer.set("Root", catalog);
        doc.trailer.set("Info", info);
        doc.trailer.set(
            "ID",
            vec![
                Object::string_literal("fingerprint"),
                Object::string_literal("fingerprint"),
            ],
        );
        doc
    }

    fn save(doc: &mut Document) -> Vec<u8> {
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    fn sticky_note() -> Object {
        Object::Dictionary(dictionary! {
            "Subtype" => "Text",
            "Contents" => Object::string_literal("ask Jane about the fee"),
        })
    }

    fn internal_link() -> Object {
        Object::Dictionary(dictionary! {
            "Subtype" => "Link",
            "A" => dictionary! { "S" => "GoTo", "D" => Object::string_literal("chapter") },
        })
    }

    fn external_link() -> Object {
        Object::Dictionary(dictionary! {
            "Subtype" => "Link",
            "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("mailto:jane@example.com") },
        })
    }

    #[test]
    fn the_record_names_each_removed_item() {
        let mut doc = document(vec![sticky_note(), internal_link(), external_link()]);
        let counts: Vec<(HiddenContent, usize)> = sanitize(&mut doc)
            .unwrap()
            .iter()
            .map(|entry| (entry.content, entry.count))
            .collect();
        assert_eq!(
            counts,
            [
                (HiddenContent::Properties, 2),
                (HiddenContent::Annotation, 1),
                (HiddenContent::Thumbnail, 1),
                (HiddenContent::Link, 1),
                (HiddenContent::Script, 1),
                (HiddenContent::Action, 1),
                (HiddenContent::DocumentId, 1),
                (HiddenContent::ApplicationData, 1),
                (HiddenContent::Outline, 1),
            ]
        );
    }

    #[test]
    fn an_unreachable_object_is_a_prior_revision() {
        let mut doc = document(Vec::new());
        doc.add_object(Object::string_literal("deleted paragraph"));
        let stripped = sanitize(&mut doc).unwrap();
        assert!(
            stripped
                .iter()
                .any(|entry| { entry.content == HiddenContent::PriorRevision && entry.count == 1 })
        );
    }

    #[test]
    fn removes_every_hidden_structure_but_internal_links() {
        let mut doc = document(vec![sticky_note(), internal_link(), external_link()]);
        assert!(!sanitize(&mut doc).unwrap().is_empty());
        let out = save(&mut doc);
        for leaked in [
            b"Jane".as_slice(),
            b"jane@example.com",
            b"app.alert",
            b"thumbnail",
            b"Illustrator",
            b"fingerprint",
        ] {
            assert!(
                !contains(&out, leaked),
                "{} survived",
                String::from_utf8_lossy(leaked)
            );
        }
        assert!(contains(&out, b"chapter"), "the internal link was removed");
        assert!(contains(&out, b"/Dests"), "named destinations were removed");
    }

    /// The record's count of `content`.
    fn count(stripped: &StrippedContent, content: HiddenContent) -> usize {
        stripped
            .iter()
            .filter(|entry| entry.content == content)
            .map(|entry| entry.count)
            .sum()
    }

    fn page(doc: &Document) -> &Dictionary {
        let id = doc.get_pages().into_values().next().unwrap();
        doc.get_dictionary(id).unwrap()
    }

    /// The catalog, page, and trailer keep only what they need: everything
    /// else goes, named by what it held.
    #[test]
    fn the_catalog_page_and_trailer_keep_only_what_they_need() {
        let mut doc = document(Vec::new());
        let catalog = doc.catalog_mut().unwrap();
        catalog.set("DSS", dictionary! { "Certs" => vec![] });
        catalog.set("Perms", dictionary! {});
        catalog.set("SpiderInfo", dictionary! {});
        catalog.set("PageLabels", dictionary! { "Nums" => vec![] });
        catalog.set("VendorPrivate", Object::string_literal("x"));
        let page_id = doc.get_pages().into_values().next().unwrap();
        let page_dict = doc.get_dictionary_mut(page_id).unwrap();
        page_dict.set("LastModified", Object::string_literal("D:20240101"));
        page_dict.set("B", vec![]);
        doc.trailer.set("DocChecksum", Object::string_literal("x"));

        let stripped = sanitize(&mut doc).unwrap();
        assert_eq!(count(&stripped, HiddenContent::Signature), 2);
        assert_eq!(count(&stripped, HiddenContent::Link), 1);
        let catalog = doc.catalog().unwrap();
        for key in [
            &b"DSS"[..],
            b"Perms",
            b"SpiderInfo",
            b"PageLabels",
            b"VendorPrivate",
        ] {
            assert!(
                catalog.get(key).is_err(),
                "{}",
                String::from_utf8_lossy(key)
            );
        }
        assert!(catalog.get(b"Pages").is_ok());
        assert!(page(&doc).get(b"LastModified").is_err() && page(&doc).get(b"B").is_err());
        assert!(page(&doc).get(b"Contents").is_ok());
        assert!(doc.trailer.get(b"DocChecksum").is_err() && doc.trailer.get(b"Root").is_ok());
    }

    /// Marked content's own text (what assistive technology reads in place of
    /// the glyphs) is never drawn or extracted, so it goes.
    #[test]
    fn marked_content_alternate_text_is_stripped() {
        let mut doc = document(Vec::new());
        let page_id = doc.get_pages().into_values().next().unwrap();
        let marked = Content {
            operations: vec![
                Operation::new(
                    "BDC",
                    vec![
                        Object::Name(b"Span".to_vec()),
                        Object::Dictionary(
                            dictionary! { "ActualText" => Object::string_literal("Jane Roe") },
                        ),
                    ],
                ),
                Operation::new("EMC", vec![]),
            ],
        };
        doc.change_page_content(page_id, marked.encode().unwrap())
            .unwrap();

        let stripped = sanitize(&mut doc).unwrap();
        assert_eq!(count(&stripped, HiddenContent::AlternativeText), 1);
        assert!(!contains(&doc.get_page_content(page_id), b"Jane Roe"));
    }

    /// A kept internal link keeps only where it goes; one that chains a further
    /// action after the jump is not internal and goes.
    #[test]
    fn a_kept_link_is_reduced_and_a_chained_one_goes() {
        let tooltip = Object::Dictionary(dictionary! {
            "Subtype" => "Link",
            "Rect" => vec![0.into(), 0.into(), 1.into(), 1.into()],
            "Dest" => Object::string_literal("chapter"),
            "Contents" => Object::string_literal("ask Jane Roe"),
            "T" => Object::string_literal("Jane Roe"),
        });
        let chained = Object::Dictionary(dictionary! {
            "Subtype" => "Link",
            "A" => dictionary! {
                "S" => "GoTo",
                "D" => Object::string_literal("chapter"),
                "Next" => dictionary! { "S" => "JavaScript", "JS" => "app.alert(1)" },
            },
        });
        let mut doc = document(vec![tooltip, chained]);
        let stripped = sanitize(&mut doc).unwrap();
        assert_eq!(count(&stripped, HiddenContent::Annotation), 1);
        assert_eq!(count(&stripped, HiddenContent::Link), 1);
        let out = save(&mut doc);
        assert!(!contains(&out, b"Jane Roe") && !contains(&out, b"app.alert"));
        assert!(contains(&out, b"chapter"));
    }

    #[test]
    fn a_sanitized_document_is_left_unchanged() {
        let mut doc = document(vec![sticky_note()]);
        assert!(!sanitize(&mut doc).unwrap().is_empty());
        assert!(sanitize(&mut doc).unwrap().is_empty());
    }
}
