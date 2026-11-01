//! The places a PDF records its embedded files: the `/EmbeddedFiles` name
//! tree, walked and edited node by node, and the `/AF` associated-file arrays.

use std::collections::BTreeSet;

use elide_core::Result;
use lopdf::{Dictionary, Document, Object, ObjectId, decode_text_string};

use crate::error::malformed;
use crate::lopdf_ext::{DictionaryExt, ObjectExt};

/// How deep the name tree is walked before it is treated as malformed: a real
/// tree is a few levels deep, a cyclic or adversarial one is not walked forever.
const MAX_TREE_DEPTH: usize = 32;

/// Where a name-tree node lives: an indirect object, or inline under the
/// catalog's `/Names` (itself inline, or the indirect object named here).
#[derive(Debug, Clone, Copy)]
pub(crate) enum Node {
    /// A node that is its own indirect object.
    Object(ObjectId),
    /// The tree's root written inline in the `/Names` dictionary, which is the
    /// catalog's own entry (`None`) or the indirect object named.
    InlineRoot(Option<ObjectId>),
}

/// One file in the tree: its name and its embedded-file stream.
#[derive(Debug)]
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) stream: ObjectId,
}

/// The `/EmbeddedFiles` name tree, read once: its nodes (so entries can be
/// dropped in place) and its file entries.
pub(crate) struct Tree {
    pub(crate) nodes: Vec<Node>,
    pub(crate) entries: Vec<Entry>,
}

impl Tree {
    /// Read the tree of `doc`, empty when the document attaches no file.
    pub(crate) fn read(doc: &Document) -> Result<Self> {
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
    pub(crate) fn drop_entries(&self, doc: &mut Document, removed: &BTreeSet<ObjectId>) {
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
        .file_name()
        .or_else(|| decode_text_string(key).ok())
        .unwrap_or_default();
    Some(Entry { name, stream })
}

/// The dictionary of the tree node `node` in `doc`.
pub(crate) fn node_dict(doc: &Document, node: Node) -> Option<&Dictionary> {
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

/// Every file associated through an `/AF` array, on any object of `doc`, in
/// object order.
pub(crate) fn associated(doc: &Document) -> Vec<Entry> {
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

/// Replace the attachment tree of `doc` with one leaf holding `pairs` (each a
/// key and its file specification), sorted by key.
pub(crate) fn write_leaf(doc: &mut Document, mut pairs: Vec<(String, Object)>) {
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    let leaf: Vec<Object> = pairs
        .into_iter()
        .flat_map(|(key, value)| [Object::string_literal(key), value])
        .collect();
    let mut root = Dictionary::new();
    root.set("Names", Object::Array(leaf));
    let names = match doc.catalog().ok().and_then(|c| c.get(b"Names").ok()) {
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
    if let Some(names) = names {
        names.set("EmbeddedFiles", Object::Dictionary(root));
    }
}
