//! Dropping pairs from a name tree's nodes: each node a
//! [`NameTree`](crate::pdf::reader::NameTree) read, rewritten in
//! [`Changes`] without them.

use hayro_syntax::object::{Array, Dict, MaybeRef, Object};

use super::{Changes, dict_value, value};
use crate::pdf::Pdf;
use crate::pdf::object::Value;
use crate::pdf::reader::{DictExt, Node, ObjectId, Pair, id};

/// Drop in `changes`, from the tree `tree` of `pdf` at each of its `nodes`,
/// every pair `keep` rejects.
pub(crate) fn retain_names(
    pdf: &Pdf,
    tree: &[u8],
    nodes: &[Node],
    keep: impl Fn(&Pair<'_>) -> bool,
    changes: &mut Changes,
) {
    for &node in nodes {
        let Some((owner, dict)) = node_dict(pdf, tree, node) else {
            continue;
        };
        let Some(names) = dict.get::<Array<'_>>(b"Names") else {
            continue;
        };
        let items: Vec<MaybeRef<Object<'_>>> = names.raw_iter().collect();
        let mut kept = Vec::with_capacity(items.len());
        let mut dropped = false;
        for pair in items.chunks(2) {
            let read = match pair {
                [key, value] => pdf.resolve(key).zip(pdf.resolve(value)),
                _ => None,
            };
            let keeps = read.is_none_or(|(key, value)| {
                keep(&Pair {
                    key,
                    value,
                    id: pair[1].as_obj_ref().map(id),
                })
            });
            if keeps {
                kept.extend(pair.iter().map(value));
            } else {
                dropped = true;
            }
        }
        if !dropped {
            continue;
        }
        let kept = Value::Array(kept);
        // The array is written where it is held: its own object, or in place
        // in the node.
        if let Some(array) = dict.reference(b"Names") {
            changes.replace(array, kept);
            continue;
        }
        match owner {
            Owner::Node(id) => {
                let mut node = dict_value(&dict);
                node.set("Names", kept);
                changes.replace(id, node);
            }
            Owner::Names { id, names } => {
                let mut names = dict_value(&names);
                if let Some(root) = names.get_mut(tree).and_then(Value::as_dict_mut) {
                    root.set("Names", kept);
                }
                changes.replace(id, names);
            }
            Owner::Catalog { id, catalog } => {
                let mut catalog = dict_value(&catalog);
                if let Some(root) = catalog
                    .get_mut(b"Names")
                    .and_then(Value::as_dict_mut)
                    .and_then(|names| names.get_mut(tree))
                    .and_then(Value::as_dict_mut)
                {
                    root.set("Names", kept);
                }
                changes.replace(id, catalog);
            }
        }
    }
}

/// The object a node's dictionary is written in.
enum Owner<'a> {
    /// The node is the object `id`.
    Node(ObjectId),
    /// The node is the tree's root in place in the `/Names` dictionary, the
    /// object `id`.
    Names { id: ObjectId, names: Dict<'a> },
    /// The node is the tree's root in place in the catalog's own `/Names`.
    Catalog { id: ObjectId, catalog: Dict<'a> },
}

/// The dictionary of the node `node` of the tree `tree`, and where it is
/// written.
fn node_dict<'a>(pdf: &'a Pdf, tree: &[u8], node: Node) -> Option<(Owner<'a>, Dict<'a>)> {
    match node {
        Node::Object(id) => Some((Owner::Node(id), pdf.object(id)?.into_dict()?)),
        Node::InlineRoot(Some(names_id)) => {
            let names = pdf.object(names_id)?.into_dict()?;
            let root = names.get::<Dict<'a>>(tree)?;
            Some((
                Owner::Names {
                    id: names_id,
                    names,
                },
                root,
            ))
        }
        Node::InlineRoot(None) => {
            let catalog = pdf.catalog().ok()?;
            let root = catalog.get::<Dict<'a>>(b"Names")?.get::<Dict<'a>>(tree)?;
            let id = pdf.file().xref().root_id();
            Some((Owner::Catalog { id, catalog }, root))
        }
    }
}
