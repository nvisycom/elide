//! [`Attachment`]: a PDF's embedded files, surfaced as content and replaced or
//! removed once redacted.
//!
//! A PDF embeds files in two places: the `/Names /EmbeddedFiles` tree (the
//! attachments a viewer lists), and the `/AF` arrays that associate a file with
//! the catalog, a page, or an XObject (the source data of a chart, a PDF/A-3
//! invoice's XML). Either way the file is content the document carries, so the
//! codec surfaces each one for its own format to decode and redact, then writes
//! the redacted bytes back into the file's stream, or drops every reference to
//! it when no codec could read it.

use std::collections::BTreeSet;
use std::fmt::Display;

use bytes::Bytes;
use elide_core::{Error, ErrorKind, Result};
use lopdf::{Dictionary, Document, Object, ObjectId, decode_text_string};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::document::{Store, save};

/// The largest an attached file may decompress to. An attachment's stream may
/// claim any size, so its decode is bounded to refuse a decompression bomb.
const MAX_ATTACHMENT_BYTES: usize = 512 * 1024 * 1024;

/// How deep the name tree is walked before it is treated as malformed: a real
/// tree is a few levels deep, a cyclic or adversarial one is not walked forever.
const MAX_TREE_DEPTH: usize = 32;

/// The identity of an attached file within a PDF: the indirect-object id of its
/// embedded-file stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct AttachmentId {
    /// The indirect object number.
    pub number: u32,
    /// The object generation.
    pub generation: u16,
}

impl AttachmentId {
    /// The attachment whose stream is object `(number, generation)`.
    #[must_use]
    pub fn new(number: u32, generation: u16) -> Self {
        Self { number, generation }
    }

    fn from_object((number, generation): ObjectId) -> Self {
        Self::new(number, generation)
    }

    fn object(self) -> ObjectId {
        (self.number, self.generation)
    }
}

/// One attached file: its stream's id, its file name, and its decoded bytes.
#[derive(Debug, Clone)]
pub(crate) struct Attachment {
    /// The attachment's identity.
    pub(crate) id: AttachmentId,
    /// The file name the document records for it.
    pub(crate) name: String,
    /// The file's bytes, decoded from the stream's filters.
    pub(crate) bytes: Bytes,
}

/// A redacted attached file: the attachment it replaces and its new bytes.
#[derive(Debug, Clone)]
pub struct AttachmentReplacement {
    /// The attachment to replace.
    pub id: AttachmentId,
    /// The redacted file's bytes.
    pub bytes: Vec<u8>,
}

/// Every embedded file of the document in `store`: the name tree's in tree
/// order, then each associated file the tree does not already hold.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if the tree or a file's stream cannot be
/// read, or [`ErrorKind::ResourceLimit`] if a file decompresses past the bound.
pub(crate) fn attachments(store: &Store) -> Result<Vec<Attachment>> {
    let doc = store.doc();
    let tree = Tree::read(doc)?;
    let mut seen: BTreeSet<ObjectId> = BTreeSet::new();
    tree.entries
        .iter()
        .chain(associated(doc).iter())
        .filter(|entry| seen.insert(entry.stream))
        .map(|entry| {
            let stream = doc
                .get_object(entry.stream)
                .and_then(Object::as_stream)
                .map_err(|e| malformed(format!("attachment stream: {e}")))?;
            let bytes = stream
                .get_plain_content_with_limit(MAX_ATTACHMENT_BYTES)
                .map_err(|e| {
                    Error::new(
                        ErrorKind::ResourceLimit,
                        format!(
                            "attachment `{}` could not be decoded within bounds: {e}",
                            entry.name
                        ),
                    )
                })?;
            Ok(Attachment {
                id: AttachmentId::from_object(entry.stream),
                name: entry.name.clone(),
                bytes: Bytes::from(bytes),
            })
        })
        .collect()
}

/// Write each replacement's bytes into its attachment's stream and drop every
/// reference to each `removed` attachment (its name-tree entry and its `/AF`
/// associations), returning the new document bytes.
///
/// # Errors
///
/// [`ErrorKind::Redaction`] if a replacement or removal names no attachment of
/// the document, or [`ErrorKind::MalformedInput`] if the document cannot be
/// re-saved.
pub(crate) fn redact_attachments(
    store: &Store,
    replacements: &[AttachmentReplacement],
    removed: &[AttachmentId],
) -> Result<Vec<u8>> {
    let mut doc = store.clone_doc();
    let tree = Tree::read(&doc)?;
    let known: BTreeSet<ObjectId> = tree
        .entries
        .iter()
        .chain(associated(&doc).iter())
        .map(|entry| entry.stream)
        .collect();
    let unknown = replacements
        .iter()
        .map(|r| r.id)
        .chain(removed.iter().copied())
        .find(|id| !known.contains(&id.object()));
    if let Some(id) = unknown {
        return Err(Error::new(
            ErrorKind::Redaction,
            format!(
                "object ({}, {}) is not an attachment of this document",
                id.number, id.generation
            ),
        ));
    }

    for replacement in replacements {
        let Ok(Object::Stream(stream)) = doc.get_object_mut(replacement.id.object()) else {
            return Err(malformed(format!(
                "attachment ({}, {}) is not a stream",
                replacement.id.number, replacement.id.generation
            )));
        };
        stream.set_plain_content(replacement.bytes.clone());
        if let Ok(Object::Dictionary(params)) = stream.dict.get_mut(b"Params") {
            params.set("Size", Object::Integer(replacement.bytes.len() as i64));
        }
        // Best effort: an uncompressible file is stored plain.
        let _ = stream.compress();
    }

    let removed: BTreeSet<ObjectId> = removed.iter().map(|id| id.object()).collect();
    if !removed.is_empty() {
        tree.drop_entries(&mut doc, &removed);
        drop_associated(&mut doc, &removed);
    }
    save(&mut doc)
}

/// The neutral name every embedded file is given: its file name is visible in
/// a viewer's attachment panel but never reaches detection.
const NEUTRAL_PREFIX: &str = "attachment-";

/// Give every embedded file a neutral name, `attachment-N` with its original
/// extension (which the codec decodes it by), and drop the portfolio and
/// related-file entries that name it too, returning how many file
/// specifications changed.
///
/// A file's name (`/F`, `/UF`, and its key in the attachment tree) is shown to
/// a reader but never extracted, so it cannot be redacted and is replaced. The
/// tree is rebuilt as one sorted leaf, since renamed keys would break its
/// order and limits. A name that is already neutral stays, so a second pass
/// changes nothing.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if the attachment tree cannot be read.
pub(crate) fn neutralize_names(doc: &mut Document) -> Result<usize> {
    let specs: Vec<ObjectId> = doc
        .objects
        .iter()
        .filter(|(_, object)| dictionary(object).is_some_and(|dict| dict.get(b"EF").is_ok()))
        .map(|(&id, _)| id)
        .collect();
    let mut edited_specs: BTreeSet<ObjectId> = BTreeSet::new();
    let mut names: Vec<(ObjectId, String)> = Vec::new();
    for (index, id) in specs.into_iter().enumerate() {
        let Some(spec) = doc.get_object_mut(id).ok().and_then(dictionary_mut) else {
            continue;
        };
        let current = spec_name(spec).unwrap_or_default();
        let name = if is_neutral(&current) {
            current
        } else {
            neutral_name(index + 1, &current)
        };
        let mut edited = false;
        for key in [&b"F"[..], b"UF"] {
            let already = spec
                .get(key)
                .ok()
                .and_then(|name| decode_text_string(name).ok())
                .is_some_and(|value| value == name);
            if !already {
                spec.set(key, Object::string_literal(name.clone()));
                edited = true;
            }
        }
        for key in [&b"CI"[..], b"RF"] {
            edited |= spec.remove(key).is_some();
        }
        if edited {
            edited_specs.insert(id);
        }
        names.push((id, name));
    }
    let renamed_keys = rebuild_tree(doc, &names, &edited_specs)?;
    Ok(edited_specs.len() + renamed_keys)
}

/// Rebuild the attachment tree as one leaf of its entries sorted by key, each
/// keyed by its file specification's neutral name from `names`, when any key
/// is not already that name. Returns how many keys changed whose file
/// specification is not among the `edited` ones (already counted).
fn rebuild_tree(
    doc: &mut Document,
    names: &[(ObjectId, String)],
    edited: &BTreeSet<ObjectId>,
) -> Result<usize> {
    let tree = Tree::read(doc)?;
    if tree.nodes.is_empty() {
        return Ok(0);
    }
    let mut renamed = 0;
    let mut any_renamed = false;
    let mut pairs: Vec<(String, Object)> = tree
        .nodes
        .iter()
        .filter_map(|&node| {
            node_dict(doc, node)?
                .get(b"Names")
                .and_then(Object::as_array)
                .ok()
        })
        .flat_map(|array| array.chunks_exact(2))
        .map(|pair| {
            let current = decode_text_string(&pair[0]).unwrap_or_default();
            let spec = match &pair[1] {
                Object::Reference(id) => names.iter().find(|(spec, _)| spec == id),
                _ => None,
            };
            let key = spec.map_or_else(|| current.clone(), |(_, name)| name.clone());
            if key != current {
                any_renamed = true;
                renamed += usize::from(spec.is_none_or(|(id, _)| !edited.contains(id)));
            }
            (key, pair[1].clone())
        })
        .collect();
    if !any_renamed {
        return Ok(0);
    }
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    let leaf: Vec<Object> = pairs
        .into_iter()
        .flat_map(|(key, value)| [Object::string_literal(key), value])
        .collect();
    let mut root = Dictionary::new();
    root.set("Names", Object::Array(leaf));
    let names_dict = match doc.catalog().ok().and_then(|c| c.get(b"Names").ok()) {
        Some(Object::Reference(id)) => {
            let id = *id;
            doc.get_object_mut(id)
                .ok()
                .and_then(|o| o.as_dict_mut().ok())
        }
        _ => doc
            .catalog_mut()
            .ok()
            .and_then(|catalog| catalog.get_mut(b"Names").ok())
            .and_then(|names| names.as_dict_mut().ok()),
    };
    if let Some(names_dict) = names_dict {
        names_dict.set("EmbeddedFiles", Object::Dictionary(root));
    }
    Ok(renamed)
}

/// A file specification's file name: its Unicode `/UF`, else its `/F`.
fn spec_name(spec: &Dictionary) -> Option<String> {
    spec.get(b"UF")
        .or_else(|_| spec.get(b"F"))
        .ok()
        .and_then(|name| decode_text_string(name).ok())
}

/// Whether `name` is already neutral: `attachment-N`, with or without an
/// extension.
fn is_neutral(name: &str) -> bool {
    let stem = name.split_once('.').map_or(name, |(stem, _)| stem);
    stem.strip_prefix(NEUTRAL_PREFIX)
        .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
}

/// The neutral name of the `index`th embedded file, keeping the extension of
/// its `original` name when it has a plain one.
fn neutral_name(index: usize, original: &str) -> String {
    let extension = original
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| {
            (1..=8).contains(&extension.len())
                && extension.bytes().all(|b| b.is_ascii_alphanumeric())
        });
    match extension {
        Some(extension) => format!("{NEUTRAL_PREFIX}{index}.{}", extension.to_ascii_lowercase()),
        None => format!("{NEUTRAL_PREFIX}{index}"),
    }
}

/// Every file associated through an `/AF` array, on any object of `doc`, in
/// object order.
fn associated(doc: &Document) -> Vec<Entry> {
    doc.objects
        .values()
        .filter_map(dictionary)
        .filter_map(|dict| dict.get(b"AF").ok())
        .filter_map(|af| associations(doc, af))
        .flat_map(|specs| {
            specs
                .iter()
                .filter_map(|spec| entry(doc, &Object::Null, spec))
        })
        .collect()
}

/// Drop every `/AF` association whose file is in `removed`, removing an array
/// left empty.
fn drop_associated(doc: &mut Document, removed: &BTreeSet<ObjectId>) {
    // Decide which arrays change while the document is readable: an
    // association resolves to its stream through a (possibly referenced) file
    // specification, which a mutable borrow would block.
    let mut edits: Vec<(ObjectId, Option<ObjectId>, Vec<Object>)> = Vec::new();
    for (&owner, object) in &doc.objects {
        let Some(af) = dictionary(object).and_then(|dict| dict.get(b"AF").ok()) else {
            continue;
        };
        let Some(specs) = associations(doc, af) else {
            continue;
        };
        let kept: Vec<Object> = specs
            .iter()
            .filter(|spec| {
                entry(doc, &Object::Null, spec).is_none_or(|e| !removed.contains(&e.stream))
            })
            .cloned()
            .collect();
        if kept.len() != specs.len() {
            edits.push((owner, af.as_reference().ok(), kept));
        }
    }
    for (owner, array, kept) in edits {
        match array {
            // The array is its own object: rewrite it, and drop it from its
            // owner once empty.
            Some(array) => {
                let empty = kept.is_empty();
                doc.objects.insert(array, Object::Array(kept));
                if empty && let Some(dict) = doc.get_object_mut(owner).ok().and_then(dictionary_mut)
                {
                    dict.remove(b"AF");
                }
            }
            None => {
                if let Some(dict) = doc.get_object_mut(owner).ok().and_then(dictionary_mut) {
                    if kept.is_empty() {
                        dict.remove(b"AF");
                    } else {
                        dict.set("AF", Object::Array(kept));
                    }
                }
            }
        }
    }
}

/// The file specifications of an `/AF` value: an inline array, or a reference
/// to one.
fn associations<'a>(doc: &'a Document, af: &'a Object) -> Option<&'a Vec<Object>> {
    match af {
        Object::Array(specs) => Some(specs),
        Object::Reference(id) => doc.get_object(*id).and_then(Object::as_array).ok(),
        _ => None,
    }
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

/// Where a name-tree node lives: an indirect object, or inline under the
/// catalog's `/Names` (itself inline, or the indirect object named here).
#[derive(Debug, Clone, Copy)]
enum Node {
    /// A node that is its own indirect object.
    Object(ObjectId),
    /// The tree's root written inline in the `/Names` dictionary, which is the
    /// catalog's own entry (`None`) or the indirect object named.
    InlineRoot(Option<ObjectId>),
}

/// One file in the tree: its name and its embedded-file stream.
#[derive(Debug)]
struct Entry {
    name: String,
    stream: ObjectId,
}

/// The `/EmbeddedFiles` name tree, read once: its nodes (so entries can be
/// dropped in place) and its file entries.
struct Tree {
    nodes: Vec<Node>,
    entries: Vec<Entry>,
}

impl Tree {
    /// Read the tree of `doc`, empty when the document attaches no file.
    fn read(doc: &Document) -> Result<Self> {
        let mut tree = Self {
            nodes: Vec::new(),
            entries: Vec::new(),
        };
        let Some(names) = doc.catalog().ok().and_then(|c| c.get(b"Names").ok()) else {
            return Ok(tree);
        };
        let (names, names_id) = match names {
            Object::Reference(id) => (doc.get_dictionary(*id).ok(), Some(*id)),
            Object::Dictionary(dict) => (Some(dict), None),
            _ => (None, None),
        };
        let Some(root) = names.and_then(|names| names.get(b"EmbeddedFiles").ok()) else {
            return Ok(tree);
        };
        let mut visited = BTreeSet::new();
        match root {
            Object::Reference(id) => tree.walk(doc, Node::Object(*id), &mut visited, 0)?,
            Object::Dictionary(_) => tree.walk(doc, Node::InlineRoot(names_id), &mut visited, 0)?,
            _ => {}
        }
        Ok(tree)
    }

    /// Record `node` and its entries, then walk its kids.
    fn walk(
        &mut self,
        doc: &Document,
        node: Node,
        visited: &mut BTreeSet<ObjectId>,
        depth: usize,
    ) -> Result<()> {
        if depth > MAX_TREE_DEPTH {
            return Err(malformed("attachment name tree is too deep"));
        }
        if let Node::Object(id) = node
            && !visited.insert(id)
        {
            return Err(malformed("attachment name tree is cyclic"));
        }
        let dict = node_dict(doc, node)
            .ok_or_else(|| malformed("unreadable attachment name tree node"))?;
        self.nodes.push(node);
        if let Ok(names) = dict.get(b"Names").and_then(Object::as_array) {
            for pair in names.chunks(2) {
                let [key, value] = pair else {
                    return Err(malformed("attachment name tree has an unpaired name"));
                };
                if let Some(entry) = entry(doc, key, value) {
                    self.entries.push(entry);
                }
            }
        }
        if let Ok(kids) = dict.get(b"Kids").and_then(Object::as_array) {
            for kid in kids {
                let id = kid
                    .as_reference()
                    .map_err(|_| malformed("attachment name tree kid is not a reference"))?;
                self.walk(doc, Node::Object(id), visited, depth + 1)?;
            }
        }
        Ok(())
    }

    /// Drop every entry whose embedded-file stream is in `removed` from the tree
    /// in `doc`.
    fn drop_entries(&self, doc: &mut Document, removed: &BTreeSet<ObjectId>) {
        // Decide which pairs each node keeps while the tree is still readable: a
        // value resolves to its stream through a (possibly referenced) file
        // specification, which a mutable borrow of the node would block.
        let kept_by_node: Vec<(Node, Vec<bool>)> = self
            .nodes
            .iter()
            .filter_map(|&node| {
                let names = node_dict(doc, node)?
                    .get(b"Names")
                    .and_then(Object::as_array)
                    .ok()?;
                let keep = names
                    .chunks(2)
                    .map(|pair| match pair {
                        [key, value] => {
                            entry(doc, key, value).is_none_or(|e| !removed.contains(&e.stream))
                        }
                        _ => true,
                    })
                    .collect();
                Some((node, keep))
            })
            .collect();
        for (node, keep) in kept_by_node {
            let Some(dict) = node_dict_mut(doc, node) else {
                continue;
            };
            let Ok(Object::Array(names)) = dict.get_mut(b"Names") else {
                continue;
            };
            let kept: Vec<Object> = names
                .chunks(2)
                .zip(&keep)
                .filter(|(_, keep)| **keep)
                .flat_map(|(pair, _)| pair.iter().cloned())
                .collect();
            *names = kept;
        }
    }
}

/// The file entry a name-tree `key` / `value` pair describes, or `None` when the
/// value is not a file specification with an embedded file.
fn entry(doc: &Document, key: &Object, value: &Object) -> Option<Entry> {
    let spec = match value {
        Object::Reference(id) => doc.get_dictionary(*id).ok()?,
        Object::Dictionary(dict) => dict,
        _ => return None,
    };
    let files = spec.get(b"EF").and_then(Object::as_dict).ok()?;
    let stream = files
        .get(b"UF")
        .or_else(|_| files.get(b"F"))
        .and_then(Object::as_reference)
        .ok()?;
    let name = spec
        .get(b"UF")
        .or_else(|_| spec.get(b"F"))
        .ok()
        .and_then(|name| decode_text_string(name).ok())
        .or_else(|| decode_text_string(key).ok())
        .unwrap_or_default();
    Some(Entry { name, stream })
}

/// The dictionary of the tree node `node` in `doc`.
fn node_dict(doc: &Document, node: Node) -> Option<&Dictionary> {
    match node {
        Node::Object(id) => doc.get_dictionary(id).ok(),
        Node::InlineRoot(names) => names_dict(doc, names)?
            .get(b"EmbeddedFiles")
            .and_then(Object::as_dict)
            .ok(),
    }
}

/// The dictionary of the tree node `node` in `doc`, mutably.
fn node_dict_mut(doc: &mut Document, node: Node) -> Option<&mut Dictionary> {
    let names = match node {
        Node::Object(id) => return doc.get_dictionary_mut(id).ok(),
        Node::InlineRoot(Some(id)) => doc.get_dictionary_mut(id).ok()?,
        Node::InlineRoot(None) => doc
            .catalog_mut()
            .ok()?
            .get_mut(b"Names")
            .and_then(Object::as_dict_mut)
            .ok()?,
    };
    names
        .get_mut(b"EmbeddedFiles")
        .and_then(Object::as_dict_mut)
        .ok()
}

/// The catalog's `/Names` dictionary: the indirect object `names`, or the
/// catalog's inline entry.
fn names_dict(doc: &Document, names: Option<ObjectId>) -> Option<&Dictionary> {
    match names {
        Some(id) => doc.get_dictionary(id).ok(),
        None => doc
            .catalog()
            .ok()?
            .get(b"Names")
            .and_then(Object::as_dict)
            .ok(),
    }
}

fn malformed(message: impl Display) -> Error {
    Error::new(ErrorKind::MalformedInput, message.to_string())
}

#[cfg(test)]
mod tests {
    use lopdf::{Stream, dictionary};

    use super::*;
    use crate::document::Pdf;

    /// A one-page PDF attaching `files` (name, bytes), split across two kid
    /// nodes of the name tree so the walk descends.
    fn with_attachments(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut doc = Document::with_version("1.7");
        let pages = doc.new_object_id();
        let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages });
        doc.objects.insert(
            pages,
            Object::Dictionary(
                dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
            ),
        );
        let mut kids = Vec::new();
        for (name, bytes) in files {
            let mut stream = Stream::new(
                dictionary! { "Type" => "EmbeddedFile", "Params" => dictionary! { "Size" => bytes.len() as i64 } },
                bytes.to_vec(),
            );
            stream.compress().unwrap();
            let stream = doc.add_object(stream);
            let spec = doc.add_object(dictionary! {
                "Type" => "Filespec",
                "F" => Object::string_literal(*name),
                "EF" => dictionary! { "F" => stream },
            });
            let kid = doc.add_object(dictionary! {
                "Names" => vec![Object::string_literal(*name), spec.into()],
            });
            kids.push(Object::Reference(kid));
        }
        let catalog = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages,
            "Names" => dictionary! { "EmbeddedFiles" => dictionary! { "Kids" => kids } },
        });
        doc.trailer.set("Root", catalog);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    fn names(pdf: &Pdf) -> Vec<(String, Vec<u8>)> {
        pdf.attachments()
            .unwrap()
            .into_iter()
            .map(|a| (a.name, a.bytes.to_vec()))
            .collect()
    }

    #[test]
    fn reads_every_attached_file_decoded() {
        let pdf = Pdf::open(&with_attachments(&[
            ("a.txt", b"Jane Roe"),
            ("b.csv", b"x,y"),
        ]))
        .unwrap();
        assert_eq!(
            names(&pdf),
            [
                ("a.txt".to_owned(), b"Jane Roe".to_vec()),
                ("b.csv".to_owned(), b"x,y".to_vec())
            ]
        );
    }

    #[test]
    fn replaces_and_removes_attached_files() {
        let pdf = Pdf::open(&with_attachments(&[
            ("a.txt", b"Jane Roe"),
            ("b.csv", b"x,y"),
        ]))
        .unwrap();
        let [a, b] = pdf.attachments().unwrap().try_into().unwrap();
        let out = pdf
            .redact_attachments(
                &[AttachmentReplacement {
                    id: a.id,
                    bytes: b"[NAME]".to_vec(),
                }],
                &[b.id],
            )
            .unwrap();
        let redacted = Pdf::open(&out).unwrap();
        assert_eq!(names(&redacted), [("a.txt".to_owned(), b"[NAME]".to_vec())]);
    }

    /// A one-page PDF whose page associates `on_page` through an inline `/AF`
    /// array and whose catalog associates `on_catalog` through a referenced
    /// one; neither file is in the attachment tree.
    fn with_associated(on_page: (&str, &[u8]), on_catalog: (&str, &[u8])) -> Vec<u8> {
        let mut doc = Document::with_version("2.0");
        let mut spec = |(name, bytes): (&str, &[u8])| {
            let stream = doc.add_object(Stream::new(
                dictionary! { "Type" => "EmbeddedFile" },
                bytes.to_vec(),
            ));
            doc.add_object(dictionary! {
                "Type" => "Filespec",
                "F" => Object::string_literal(name),
                "EF" => dictionary! { "F" => stream },
            })
        };
        let page_spec = spec(on_page);
        let catalog_spec = spec(on_catalog);
        let catalog_af = doc.add_object(Object::Array(vec![catalog_spec.into()]));
        let pages = doc.new_object_id();
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages, "AF" => vec![page_spec.into()],
        });
        doc.objects.insert(
            pages,
            Object::Dictionary(
                dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
            ),
        );
        let catalog = doc.add_object(dictionary! {
            "Type" => "Catalog", "Pages" => pages, "AF" => catalog_af,
        });
        doc.trailer.set("Root", catalog);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    #[test]
    fn associated_files_are_surfaced_redacted_and_dropped() {
        let pdf = Pdf::open(&with_associated(
            ("invoice.xml", b"<buyer>Jane Roe</buyer>"),
            ("source.csv", b"Jane,Roe"),
        ))
        .unwrap();
        let mut files = pdf.attachments().unwrap();
        files.sort_by(|a, b| a.name.cmp(&b.name));
        let [invoice, source] = files.try_into().unwrap();
        assert_eq!(invoice.name, "invoice.xml");

        let out = pdf
            .redact_attachments(
                &[AttachmentReplacement {
                    id: invoice.id,
                    bytes: b"<buyer>[NAME]</buyer>".to_vec(),
                }],
                &[source.id],
            )
            .unwrap();
        let redacted = Pdf::open(&out).unwrap();
        assert_eq!(
            names(&redacted),
            [("invoice.xml".to_owned(), b"<buyer>[NAME]</buyer>".to_vec())]
        );
        // The catalog's association, left empty, is gone with its file.
        let doc = Document::load_mem(&out).unwrap();
        assert!(doc.catalog().unwrap().get(b"AF").is_err());
    }

    /// A file name a viewer shows but detection never sees goes: each file is
    /// renamed `attachment-N` with its extension, its tree key with it, and a
    /// second pass changes nothing.
    #[test]
    fn attached_file_names_are_made_neutral() {
        let bytes = with_attachments(&[
            ("Jane_Roe_passport.pdf", b"pdf bytes"),
            ("notes.txt", b"Jane Roe"),
        ]);
        let mut doc = Document::load_mem(&bytes).unwrap();
        assert_eq!(neutralize_names(&mut doc).unwrap(), 2);
        assert_eq!(neutralize_names(&mut doc).unwrap(), 0);

        // The replaced tree nodes are unreachable; the save prunes them.
        let out = save(&mut doc).unwrap();
        assert!(!out.windows(8).any(|w| w == b"Jane_Roe"));
        let mut names = names(&Pdf::open(&out).unwrap());
        names.sort();
        assert_eq!(
            names,
            [
                ("attachment-1.pdf".to_owned(), b"pdf bytes".to_vec()),
                ("attachment-2.txt".to_owned(), b"Jane Roe".to_vec()),
            ]
        );
    }

    #[test]
    fn refuses_an_unknown_attachment() {
        let pdf = Pdf::open(&with_attachments(&[("a.txt", b"x")])).unwrap();
        let unknown = AttachmentId::new(999, 0);
        assert!(pdf.redact_attachments(&[], &[unknown]).is_err());
    }
}
