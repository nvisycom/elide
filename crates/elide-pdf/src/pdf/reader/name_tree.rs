//! Reading name trees (ISO 32000-2, 7.9.6): the trees under the catalog's
//! `/Names`, node by node.

use std::collections::BTreeSet;

use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::pdf::reader::DocumentExt;

/// How deep a name tree is walked: a real tree is a few levels deep, a cyclic
/// or adversarial one is not walked forever.
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

/// One name tree of a document, read once: its nodes, so pairs can be
/// dropped in place, and its key/value pairs in tree order.
pub(crate) struct NameTree<'a> {
    pub(crate) nodes: Vec<Node>,
    pub(crate) pairs: Vec<(&'a Object, &'a Object)>,
    /// Whether every node was read: none failed to resolve, recurred, lay
    /// past [`MAX_TREE_DEPTH`], or held an unpaired name. A caller that must
    /// see every pair refuses an incomplete tree; one that may miss a pair
    /// reads what was.
    pub(crate) complete: bool,
}

impl<'a> NameTree<'a> {
    /// The tree `tree` (`/EmbeddedFiles`, `/Dests`, ...) of `doc`'s
    /// `/Names`, empty when the document has none.
    pub(crate) fn read(doc: &'a Document, tree: &[u8]) -> Self {
        let mut out = Self {
            nodes: Vec::new(),
            pairs: Vec::new(),
            complete: true,
        };
        let Some(names) = doc.catalog().ok().and_then(|c| c.get(b"Names").ok()) else {
            return out;
        };
        let (names, names_id) = match names {
            Object::Reference(id) => (doc.get_dictionary(*id).ok(), Some(*id)),
            Object::Dictionary(dict) => (Some(dict), None),
            _ => (None, None),
        };
        let Some(root) = names.and_then(|names| names.get(tree).ok()) else {
            return out;
        };
        let root = match root {
            Object::Reference(id) => Node::Object(*id),
            Object::Dictionary(_) => Node::InlineRoot(names_id),
            _ => {
                out.complete = false;
                return out;
            }
        };
        out.walk(doc, tree, root, &mut BTreeSet::new(), 0);
        out
    }

    /// Record `node` and its pairs, then walk its kids.
    fn walk(
        &mut self,
        doc: &'a Document,
        tree: &[u8],
        node: Node,
        visited: &mut BTreeSet<ObjectId>,
        depth: usize,
    ) {
        if depth > MAX_TREE_DEPTH {
            self.complete = false;
            return;
        }
        if let Node::Object(id) = node
            && !visited.insert(id)
        {
            self.complete = false;
            return;
        }
        let Some(dict) = node_dict(doc, tree, node) else {
            self.complete = false;
            return;
        };
        self.nodes.push(node);
        if let Ok(names) = dict.get(b"Names") {
            let Some(names) = self.array(doc, names, visited) else {
                return;
            };
            let (pairs, unpaired) = names.as_chunks::<2>();
            self.complete &= unpaired.is_empty();
            self.pairs
                .extend(pairs.iter().map(|[key, value]| (key, value)));
        }
        if let Ok(kids) = dict.get(b"Kids") {
            let Some(kids) = self.array(doc, kids, visited) else {
                return;
            };
            for kid in kids {
                match kid.as_reference() {
                    Ok(id) => self.walk(doc, tree, Node::Object(id), visited, depth + 1),
                    Err(_) => self.complete = false,
                }
            }
        }
    }

    /// The array `value` resolves to, read once: an array object another
    /// node already named, which no real tree shares, is not read again, so
    /// a tree that shares one among many nodes costs its size once.
    fn array(
        &mut self,
        doc: &'a Document,
        value: &'a Object,
        visited: &mut BTreeSet<ObjectId>,
    ) -> Option<&'a Vec<Object>> {
        let array = match value {
            Object::Reference(id) if !visited.insert(*id) => None,
            value => doc.resolve_array(value),
        };
        self.complete &= array.is_some();
        array
    }
}

/// The dictionary of the node `node` of the tree `tree` in `doc`.
pub(in crate::pdf) fn node_dict<'a>(
    doc: &'a Document,
    tree: &[u8],
    node: Node,
) -> Option<&'a Dictionary> {
    match node {
        Node::Object(id) => doc.get_dictionary(id).ok(),
        Node::InlineRoot(names) => names_dict(doc, names)?
            .get(tree)
            .and_then(Object::as_dict)
            .ok(),
    }
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

#[cfg(test)]
mod tests {
    use lopdf::dictionary;

    use super::*;
    use crate::fixtures::with_pages;

    /// A document whose `/Dests` tree has a kid that does not resolve, then
    /// a leaf held in a referenced `/Names` array.
    fn document() -> Document {
        let mut doc = Document::with_version("1.7");
        with_pages(&mut doc, [Dictionary::new()]);
        let names = doc.add_object(Object::Array(vec![
            Object::string_literal("a"),
            1.into(),
            Object::string_literal("b"),
            2.into(),
        ]));
        let leaf = doc.add_object(dictionary! { "Names" => names });
        doc.catalog_mut().unwrap().set(
            "Names",
            dictionary! {
                "Dests" => dictionary! { "Kids" => vec![Object::Reference((9999, 0)), leaf.into()] },
            },
        );
        doc
    }

    #[test]
    fn a_node_that_does_not_resolve_leaves_the_tree_incomplete() {
        let doc = document();
        let tree = NameTree::read(&doc, b"Dests");
        assert!(!tree.complete);
        let keys: Vec<&Object> = tree.pairs.iter().map(|(key, _)| *key).collect();
        assert_eq!(
            keys,
            [&Object::string_literal("a"), &Object::string_literal("b")]
        );
    }

    /// An array shared among many nodes is read once, its pairs taken once.
    #[test]
    fn a_shared_array_is_read_once() {
        let mut doc = Document::with_version("1.7");
        with_pages(&mut doc, [Dictionary::new()]);
        let names = doc.add_object(Object::Array(vec![Object::string_literal("a"), 1.into()]));
        let leaves: Vec<Object> = (0..3)
            .map(|_| doc.add_object(dictionary! { "Names" => names }).into())
            .collect();
        let kids = doc.add_object(Object::Array(leaves));
        let parents: Vec<Object> = (0..3)
            .map(|_| doc.add_object(dictionary! { "Kids" => kids }).into())
            .collect();
        doc.catalog_mut().unwrap().set(
            "Names",
            dictionary! { "Dests" => dictionary! { "Kids" => parents } },
        );
        let tree = NameTree::read(&doc, b"Dests");
        assert!(!tree.complete);
        assert_eq!(tree.pairs.len(), 1);
    }
}
