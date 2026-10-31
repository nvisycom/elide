//! Remove a PDF's hidden content before its text is extracted.
//!
//! A PDF carries content outside its page content streams that a reader of the
//! page never sees, or that detection never reads. The sanitizer removes it at
//! decode, so detection runs over exactly what will ship:
//!
//! - **Metadata**: the trailer `/Info` dictionary, every `/Metadata` XMP
//!   stream (the catalog's, a page's, an image's), and an attachment's
//!   description and timestamps.
//! - **Annotations**, every one but an internal link: comments, highlights,
//!   stamps, form widgets, file attachments, and links that leave the document.
//!
//! A PDF surfaces neither its metadata nor its annotations to detection, so
//! both are stripped whatever the
//! [`HiddenContentPolicy`](elide_core::redaction::HiddenContentPolicy) asks:
//! a format that cannot redact hidden content strips it.
//! - **Active content**: document and page actions (`/OpenAction`, `/AA`) and
//!   the JavaScript name tree.
//! - **Structures detection does not read**: interactive form values
//!   (`/AcroForm`), the outline (`/Outlines`), the logical structure tree and
//!   its alternate text (`/StructTreeRoot`), article threads, associated files
//!   outside the attachment tree, page thumbnails, and application private data
//!   (`/PieceInfo`).
//! - **Hidden layers**: optional-content groups are dissolved, so content that
//!   was hidden behind a layer is extracted (and redacted) like any other.
//! - **History and fingerprints**: the trailer `/ID`, and every object no
//!   longer reachable from the document (a previous revision's content).
//!
//! Each removal drops the reference; a final prune deletes every object no
//! surviving object reaches, so a removed structure's whole subtree goes without
//! touching anything still in use.

use std::collections::BTreeSet;
use std::fmt;

use elide_core::{Error, ErrorKind, Result};
use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::document::{Store, is_xref_stream, save};

/// The catalog entries removed.
const CATALOG_REMOVED: [&[u8]; 8] = [
    b"AcroForm",
    b"Outlines",
    b"OpenAction",
    b"OCProperties",
    b"StructTreeRoot",
    b"MarkInfo",
    b"Threads",
    b"URI",
];

/// The name-tree entries kept in the catalog's `/Names`: named destinations
/// (navigation) and the attachments, which are surfaced as content.
const NAMES_KEPT: [&[u8]; 2] = [b"Dests", b"EmbeddedFiles"];

/// The keys removed from every dictionary, each named by
/// the hidden content it holds.
const EVERYWHERE_REMOVED: [(&str, &[u8]); 5] = [
    ("private data", b"PieceInfo"),
    ("layers", b"OC"),
    ("actions", b"AA"),
    ("thumbnails", b"Thumb"),
    ("associated files", b"AF"),
];

/// What a sanitize pass removed, by kind; empty when the document carried no
/// hidden content.
#[derive(Debug, Default)]
pub(crate) struct Removed(BTreeSet<&'static str>);

impl Removed {
    /// Record that hidden content of kind `what` was found, when `found` holds.
    fn note(&mut self, what: &'static str, found: bool) {
        if found {
            self.0.insert(what);
        }
    }

    /// Whether nothing was removed.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for Removed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kinds: Vec<&str> = self.0.iter().copied().collect();
        f.write_str(&kinds.join(", "))
    }
}

/// The document in `store` with its hidden content removed, as
/// new bytes, or `None` when it carries none.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if the sanitized document cannot be saved.
pub(crate) fn sanitize_store(store: &Store) -> Result<Option<Vec<u8>>> {
    let mut doc = store.clone_doc();
    if sanitize(&mut doc).is_empty() {
        return Ok(None);
    }
    save(&mut doc).map(Some)
}

/// Check that the document in `store` carries no hidden content.
///
/// # Errors
///
/// [`ErrorKind::Sanitization`] naming what survived.
pub(crate) fn check_store(store: &Store) -> Result<()> {
    let removed = sanitize(&mut store.clone_doc());
    if removed.is_empty() {
        return Ok(());
    }
    Err(Error::new(
        ErrorKind::Sanitization,
        format!("encoded PDF still carries hidden content: {removed}"),
    ))
}

/// Remove `doc`'s hidden content, returning what was removed.
pub(crate) fn sanitize(doc: &mut Document) -> Removed {
    let mut removed = Removed::default();

    for page in doc.get_pages().into_values() {
        removed.note("annotations", keep_internal_links(doc, page));
    }
    removed.note("catalog structures", sanitize_catalog(doc));

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
        for (what, key) in EVERYWHERE_REMOVED {
            removed.note(what, remove_keys(dict, &[key]));
        }
        removed.note("metadata", remove_keys(dict, &[b"Metadata"]));
        removed.note("metadata", sanitize_attachment_metadata(dict));
        if resources.contains(id) {
            removed.note("layers", remove_keys(dict, &[b"Properties"]));
        }
        if let Ok(Object::Dictionary(inline)) = dict.get_mut(b"Resources") {
            removed.note("layers", remove_keys(inline, &[b"Properties"]));
        }
    }

    removed.note("metadata", remove_keys(&mut doc.trailer, &[b"Info"]));
    removed.note("file identifier", remove_keys(&mut doc.trailer, &[b"ID"]));
    // A cross-reference stream is file structure the next save rewrites, not
    // content: a loaded one is a stale copy, dropped without counting as hidden
    // content before the prune.
    doc.objects.retain(|_, object| !is_xref_stream(object));
    removed.note("unreachable objects", !doc.prune_objects().is_empty());
    removed
}

/// Remove every annotation of `page` but its internal links, returning whether
/// any was removed.
fn keep_internal_links(doc: &mut Document, page: ObjectId) -> bool {
    let annotations: Vec<Object> = match doc.get_dictionary(page).and_then(|p| p.get(b"Annots")) {
        Ok(Object::Reference(id)) => match doc.get_object(*id).and_then(Object::as_array) {
            Ok(array) => array.clone(),
            Err(_) => Vec::new(),
        },
        Ok(Object::Array(array)) => array.clone(),
        _ => return false,
    };
    let kept: Vec<Object> = annotations
        .iter()
        .filter(|annotation| {
            let dict = match annotation {
                Object::Reference(id) => doc.get_dictionary(*id).ok(),
                Object::Dictionary(dict) => Some(dict),
                _ => None,
            };
            dict.is_some_and(is_internal_link)
        })
        .cloned()
        .collect();
    if kept.len() == annotations.len() {
        return false;
    }
    let Some(page) = doc
        .get_object_mut(page)
        .ok()
        .and_then(|o| o.as_dict_mut().ok())
    else {
        return false;
    };
    if kept.is_empty() {
        page.remove(b"Annots");
    } else {
        page.set("Annots", Object::Array(kept));
    }
    true
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
/// ones, returning whether anything was removed.
fn sanitize_catalog(doc: &mut Document) -> bool {
    let names = doc
        .catalog()
        .ok()
        .and_then(|catalog| catalog.get(b"Names").ok())
        .cloned();
    let mut changed = match names {
        Some(Object::Reference(id)) => doc
            .get_object_mut(id)
            .ok()
            .and_then(|o| o.as_dict_mut().ok())
            .is_some_and(retain_kept_names),
        _ => false,
    };
    let Ok(catalog) = doc.catalog_mut() else {
        return changed;
    };
    if let Ok(Object::Dictionary(names)) = catalog.get_mut(b"Names") {
        changed |= retain_kept_names(names);
    }
    changed | remove_keys(catalog, &CATALOG_REMOVED)
}

/// Drop every entry of a `/Names` dictionary but the kept name trees,
/// returning whether any was dropped.
fn retain_kept_names(names: &mut Dictionary) -> bool {
    let dropped: Vec<Vec<u8>> = names
        .iter()
        .map(|(key, _)| key.clone())
        .filter(|key| !NAMES_KEPT.contains(&key.as_slice()))
        .collect();
    for key in &dropped {
        names.remove(key);
    }
    !dropped.is_empty()
}

/// Remove an attachment file specification's description and its embedded
/// file's timestamps and checksum, returning whether any was removed.
fn sanitize_attachment_metadata(dict: &mut Dictionary) -> bool {
    let mut changed = false;
    if dict.has_type(b"Filespec") {
        changed |= remove_keys(dict, &[b"Desc"]);
    }
    if dict.has_type(b"EmbeddedFile")
        && let Ok(Object::Dictionary(params)) = dict.get_mut(b"Params")
    {
        changed |= remove_keys(params, &[b"CreationDate", b"ModDate", b"CheckSum", b"Mac"]);
    }
    changed
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

/// Remove `keys` from `dict`, returning whether any was present.
fn remove_keys(dict: &mut Dictionary, keys: &[&[u8]]) -> bool {
    keys.iter()
        .fold(false, |changed, key| dict.remove(key).is_some() | changed)
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
