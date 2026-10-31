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
//!   markup annotation, a form widget, a link that leaves the document, or rich
//!   media.
//! - **Active content**: document and page actions (`/OpenAction`, `/AA`), and
//!   the JavaScript and rendition name trees.
//! - **Content no extraction reads**: interactive forms (`/AcroForm`), the
//!   outline and article threads, the logical structure tree and its
//!   alternate text, optional-content layers, and associated files outside the
//!   attachment tree.
//! - **Previews and fingerprints**: page thumbnails, application private data
//!   (`/PieceInfo`), and the trailer `/ID`.
//! - **Prior revisions**: every object no longer reachable from the document,
//!   a previous revision's content.
//!
//! A PDF surfaces neither its metadata nor its annotations to detection, so
//! both are stripped whatever the
//! [`HiddenContentPolicy`](elide_core::redaction::HiddenContentPolicy) asks: a
//! format that cannot redact hidden content strips it.
//!
//! Each removal drops the reference; a final prune deletes every object no
//! surviving object reaches, so a removed structure's whole subtree goes without
//! touching anything still in use.

use std::collections::BTreeSet;

use elide_codec::StrippedContent;
use elide_core::redaction::HiddenContent;
use elide_core::{Error, ErrorKind, Result};
use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::document::{Store, is_xref_stream, save};

/// The catalog entries removed, each with the hidden content it holds.
const CATALOG_REMOVED: [(&[u8], HiddenContent); 8] = [
    (b"AcroForm", HiddenContent::FormField),
    (b"Outlines", HiddenContent::Outline),
    (b"Threads", HiddenContent::Outline),
    (b"OpenAction", HiddenContent::Action),
    (b"OCProperties", HiddenContent::Layer),
    (b"StructTreeRoot", HiddenContent::AlternativeText),
    (b"MarkInfo", HiddenContent::AlternativeText),
    (b"URI", HiddenContent::Link),
];

/// The name-tree entries kept in the catalog's `/Names`: named destinations
/// (navigation) and the attachments, which are surfaced as content.
const NAMES_KEPT: [&[u8]; 2] = [b"Dests", b"EmbeddedFiles"];

/// The keys removed from every dictionary, each with the hidden content it
/// holds.
const EVERYWHERE_REMOVED: [(&[u8], HiddenContent); 6] = [
    (b"PieceInfo", HiddenContent::ApplicationData),
    (b"OC", HiddenContent::Layer),
    (b"AA", HiddenContent::Action),
    (b"Thumb", HiddenContent::Thumbnail),
    (b"AF", HiddenContent::Unreadable),
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
    let stripped = sanitize(&mut doc);
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
    let survived = sanitize(&mut store.clone_doc());
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
pub(crate) fn sanitize(doc: &mut Document) -> StrippedContent {
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

    for page in doc.get_pages().into_values() {
        for content in keep_internal_links(doc, page) {
            stripped.record(content, None, 1);
        }
    }
    sanitize_catalog(doc, &mut stripped);

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
        if resources.contains(id) {
            stripped.record(HiddenContent::Layer, None, removed(dict, b"Properties"));
        }
        if let Ok(Object::Dictionary(inline)) = dict.get_mut(b"Resources") {
            stripped.record(HiddenContent::Layer, None, removed(inline, b"Properties"));
        }
    }

    stripped.record(
        HiddenContent::Properties,
        None,
        removed(&mut doc.trailer, b"Info"),
    );
    stripped.record(
        HiddenContent::DocumentId,
        None,
        removed(&mut doc.trailer, b"ID"),
    );
    doc.prune_objects();
    stripped
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
    if dropped.is_empty() {
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
/// with a `/Dest`, or whose action is an internal `/GoTo`.
fn is_internal_link(annotation: &Dictionary) -> bool {
    let is_link = annotation
        .get(b"Subtype")
        .and_then(Object::as_name)
        .is_ok_and(|subtype| subtype == b"Link");
    if !is_link {
        return false;
    }
    match annotation.get(b"A").and_then(Object::as_dict) {
        Ok(action) => action
            .get(b"S")
            .and_then(Object::as_name)
            .is_ok_and(|kind| kind == b"GoTo"),
        Err(_) => annotation.get(b"Dest").is_ok(),
    }
}

/// Remove the catalog's hidden structures and every name tree but the kept
/// ones, recording each in `stripped`.
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
    for (key, content) in CATALOG_REMOVED {
        stripped.record(content, None, removed(catalog, key));
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
        let stripped = sanitize(&mut doc);
        assert!(
            stripped
                .iter()
                .any(|entry| { entry.content == HiddenContent::PriorRevision && entry.count == 1 })
        );
    }

    #[test]
    fn removes_every_hidden_structure_but_internal_links() {
        let mut doc = document(vec![sticky_note(), internal_link(), external_link()]);
        assert!(!sanitize(&mut doc).is_empty());
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

    #[test]
    fn a_sanitized_document_is_left_unchanged() {
        let mut doc = document(vec![sticky_note()]);
        assert!(!sanitize(&mut doc).is_empty());
        assert!(sanitize(&mut doc).is_empty());
    }
}
