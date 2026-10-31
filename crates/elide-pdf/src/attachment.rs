//! [`Attachment`]: a PDF's embedded files, the `/Names /EmbeddedFiles` tree,
//! surfaced as content and replaced or removed once redacted.
//!
//! An attached file is content the document carries, so the codec surfaces each
//! one for its own format to decode and redact, then writes the redacted bytes
//! back into the file's stream, or drops the file from the tree when no codec
//! could read it.

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

/// Every attached file of the document in `store`, in name-tree order.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`] if the tree or a file's stream cannot be
/// read, or [`ErrorKind::ResourceLimit`] if a file decompresses past the bound.
pub(crate) fn attachments(store: &Store) -> Result<Vec<Attachment>> {
    let doc = store.doc();
    let tree = Tree::read(doc)?;
    tree.entries
        .iter()
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

/// Write each replacement's bytes into its attachment's stream and drop each
/// `removed` attachment from the tree, returning the new document bytes.
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
    let known: BTreeSet<ObjectId> = tree.entries.iter().map(|entry| entry.stream).collect();
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
    }
    save(&mut doc)
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

    #[test]
    fn refuses_an_unknown_attachment() {
        let pdf = Pdf::open(&with_attachments(&[("a.txt", b"x")])).unwrap();
        let unknown = AttachmentId::new(999, 0);
        assert!(pdf.redact_attachments(&[], &[unknown]).is_err());
    }
}
