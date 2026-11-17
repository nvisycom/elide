//! The places a PDF records its embedded files: the `/EmbeddedFiles` name
//! tree and the `/AF` associated-file arrays.

use std::collections::BTreeSet;

use elide_core::{Error, Result};
use lopdf::{Dictionary, Document, Object, ObjectId, decode_text_string};

use crate::document::name_tree::{self, NameTree, Node};
use crate::document::object::{DictionaryExt, DocumentExt, ObjectExt};

/// The name tree of embedded files under the catalog's `/Names`.
const EMBEDDED_FILES: &[u8] = b"EmbeddedFiles";

/// One file in the tree: its name and its embedded-file stream.
#[derive(Debug)]
pub(crate) struct Entry<'a> {
    pub(crate) name: String,
    pub(crate) stream: ObjectId,
    /// The file specification that names it.
    pub(crate) spec: &'a Dictionary,
}

/// The `/EmbeddedFiles` name tree, read once: its nodes (so entries can be
/// dropped in place) and its file entries.
pub(crate) struct Tree<'a> {
    pub(crate) nodes: Vec<Node>,
    pub(crate) entries: Vec<Entry<'a>>,
}

impl<'a> Tree<'a> {
    /// Read the tree of `doc`, empty when the document attaches no file.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// a node of the tree cannot be read, so a file under it would go unseen.
    pub(crate) fn read(doc: &'a Document) -> Result<Self> {
        let tree = NameTree::read(doc, EMBEDDED_FILES);
        if !tree.complete {
            return Err(Error::malformed(
                "attachment name tree cannot be read whole",
            ));
        }
        let entries = tree
            .pairs
            .into_iter()
            .filter_map(|(key, value)| entry(doc, key, value))
            .collect();
        Ok(Self {
            nodes: tree.nodes,
            entries,
        })
    }
}

/// Drop every entry whose embedded-file stream is in `removed` from the tree
/// `nodes` of `doc`.
pub(crate) fn drop_entries(doc: &mut Document, nodes: &[Node], removed: &BTreeSet<ObjectId>) {
    name_tree::retain(doc, EMBEDDED_FILES, nodes, |doc, key, value| {
        entry(doc, key, value).is_none_or(|entry| !removed.contains(&entry.stream))
    });
}

/// The file entry a name-tree `key` / `value` pair describes, or `None` when the
/// value is not a file specification with an embedded file.
fn entry<'a>(doc: &'a Document, key: &Object, value: &'a Object) -> Option<Entry<'a>> {
    let spec = match value {
        Object::Reference(id) => doc.get_dictionary(*id).ok()?,
        Object::Dictionary(dict) => dict,
        _ => return None,
    };
    let files = doc.resolve_dict(spec.get(b"EF").ok()?)?;
    let stream = files
        .get(b"UF")
        .or_else(|_| files.get(b"F"))
        .and_then(Object::as_reference)
        .ok()?;
    let name = spec
        .file_name()
        .or_else(|| decode_text_string(key).ok())
        .unwrap_or_default();
    Some(Entry { name, stream, spec })
}

/// Every file associated through an `/AF` array, on any object of `doc`, in
/// object order.
pub(crate) fn associated(doc: &Document) -> Vec<Entry<'_>> {
    doc.objects
        .values()
        .filter_map(ObjectExt::dict)
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
pub(crate) fn drop_associated(doc: &mut Document, removed: &BTreeSet<ObjectId>) {
    // Decide which arrays change while the document is readable: an
    // association resolves to its stream through a (possibly referenced) file
    // specification, which a mutable borrow would block.
    let mut edits: Vec<(ObjectId, Option<ObjectId>, Vec<Object>)> = Vec::new();
    for (&owner, object) in &doc.objects {
        let Some(af) = object.dict().and_then(|dict| dict.get(b"AF").ok()) else {
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
                if empty
                    && let Some(dict) = doc.get_object_mut(owner).ok().and_then(ObjectExt::dict_mut)
                {
                    dict.remove(b"AF");
                }
            }
            None => {
                if let Some(dict) = doc.get_object_mut(owner).ok().and_then(ObjectExt::dict_mut) {
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
