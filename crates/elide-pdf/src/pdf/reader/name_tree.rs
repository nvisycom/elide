//! Reading name trees (7.9.6): the trees under the catalog's `/Names`,
//! node by node.

use std::collections::BTreeSet;

use hayro_syntax::object::{Array, Dict, MaybeRef, Object};

use crate::pdf::Pdf;
use crate::pdf::reader::{DictExt, ObjectId, id};

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

/// One key and value of a name tree: the key, the value, and the object the
/// value is when it is one of its own.
#[derive(Debug, Clone)]
pub(crate) struct Pair<'a> {
    pub(crate) key: Object<'a>,
    pub(crate) value: Object<'a>,
    pub(crate) id: Option<ObjectId>,
}

/// One name tree of a document, read once: its nodes, so pairs can be
/// dropped in place, and its key/value pairs in tree order.
pub(crate) struct NameTree<'a> {
    pub(crate) nodes: Vec<Node>,
    pub(crate) pairs: Vec<Pair<'a>>,
    /// Whether every node was read: none failed to resolve, recurred, lay
    /// past [`MAX_TREE_DEPTH`], or held an unpaired name. A caller that must
    /// see every pair refuses an incomplete tree; one that may miss a pair
    /// reads what was.
    pub(crate) complete: bool,
}

impl<'a> NameTree<'a> {
    /// The tree `tree` (`/EmbeddedFiles`, `/Dests`, ...) of `pdf`'s
    /// `/Names`, empty when the document has none.
    pub(crate) fn read(pdf: &'a Pdf, tree: &[u8]) -> Self {
        let mut reader = Reader {
            pdf,
            visited: BTreeSet::new(),
            out: Self {
                nodes: Vec::new(),
                pairs: Vec::new(),
                complete: true,
            },
        };
        let Ok(catalog) = pdf.catalog() else {
            return reader.out;
        };
        let Some(names) = catalog.get::<Dict<'a>>(b"Names") else {
            return reader.out;
        };
        let names_id = catalog.reference(b"Names");
        match names.raw(tree) {
            None => {}
            // A root that is null, or refers to null or nothing, is no tree.
            Some(MaybeRef::NotRef(Object::Null(_))) => {}
            Some(MaybeRef::Ref(_)) => match names.get::<Object<'a>>(tree) {
                None | Some(Object::Null(_)) => {}
                Some(Object::Dict(root)) => {
                    let node = names
                        .reference(tree)
                        .map_or(Node::InlineRoot(names_id), Node::Object);
                    reader.walk(node, root, 0);
                }
                Some(_) => reader.out.complete = false,
            },
            Some(MaybeRef::NotRef(Object::Dict(root))) => {
                reader.walk(Node::InlineRoot(names_id), root, 0);
            }
            Some(MaybeRef::NotRef(_)) => reader.out.complete = false,
        }
        reader.out
    }
}

/// One name tree being read: the document, the objects already read, so
/// none is read twice, and the tree as read so far.
struct Reader<'a> {
    pdf: &'a Pdf,
    visited: BTreeSet<ObjectId>,
    out: NameTree<'a>,
}

impl<'a> Reader<'a> {
    /// Record `node`, whose dictionary is `dict`, `depth` levels down, and
    /// its pairs, then walk its kids.
    fn walk(&mut self, node: Node, dict: Dict<'a>, depth: usize) {
        if depth > MAX_TREE_DEPTH {
            self.out.complete = false;
            return;
        }
        if let Node::Object(id) = node
            && !self.visited.insert(id)
        {
            self.out.complete = false;
            return;
        }
        self.out.nodes.push(node);
        // A null entry, or one referring to null, is no entry; so is a
        // `/Names` referring to nothing, which holds no pairs. `/Kids`
        // referring to nothing loses a subtree, so it is read and fails.
        if !matches!(
            dict.get::<Object<'a>>(b"Names"),
            None | Some(Object::Null(_))
        ) && let Some(names) = self.array(&dict, b"Names")
        {
            let items: Vec<MaybeRef<Object<'a>>> = names.raw_iter().collect();
            let (pairs, unpaired) = items.as_chunks::<2>();
            self.out.complete &= unpaired.is_empty();
            for [key, value] in pairs {
                let (Some((key, _)), Some((value, id))) = (self.item(key), self.item(value)) else {
                    self.out.complete = false;
                    continue;
                };
                self.out.pairs.push(Pair { key, value, id });
            }
        }
        let kids = dict.raw(b"Kids");
        let null = matches!(dict.get::<Object<'a>>(b"Kids"), Some(Object::Null(_)));
        if kids.is_none() || null {
            return;
        }
        let Some(kids) = self.array(&dict, b"Kids") else {
            return;
        };
        for kid in kids.raw_iter() {
            let MaybeRef::Ref(reference) = kid else {
                self.out.complete = false;
                continue;
            };
            let kid_id = id(reference);
            match self.pdf.object(kid_id).and_then(Object::into_dict) {
                Some(kid) => self.walk(Node::Object(kid_id), kid, depth + 1),
                None => self.out.complete = false,
            }
        }
    }

    /// The value `item` holds, and the object it is when it is one of its
    /// own; `None` when it refers to nothing.
    fn item(&self, item: &MaybeRef<Object<'a>>) -> Option<(Object<'a>, Option<ObjectId>)> {
        Some((self.pdf.resolve(item)?, item.as_obj_ref().map(id)))
    }

    /// The array `dict` holds under `key`, read once: an array object
    /// another node already named, which no real tree shares, is not read
    /// again, so a tree that shares one among many nodes costs its size
    /// once.
    fn array(&mut self, dict: &Dict<'a>, key: &[u8]) -> Option<Array<'a>> {
        let array = match dict.reference(key) {
            Some(id) if !self.visited.insert(id) => None,
            _ => dict.get::<Array<'a>>(key),
        };
        self.out.complete &= array.is_some();
        array
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{catalog, open, with_pages};
    use crate::pdf::object::{Dictionary, Id, Value, dict};
    use crate::pdf::writer::Document;

    /// A document whose `/Dests` tree has a kid that does not resolve, then
    /// a leaf held in a referenced `/Names` array.
    fn document() -> Document {
        let mut doc = Document::new((1, 7));
        with_pages(&mut doc, [Dictionary::new()]);
        let names = doc.add(Value::Array(vec![
            Value::string("a"),
            1.into(),
            Value::string("b"),
            2.into(),
        ]));
        let leaf = doc.add(dict! { "Names" => names });
        catalog(&mut doc).set(
            "Names",
            dict! {
                "Dests" => dict! { "Kids" => vec![Value::Ref(Id(9999)), leaf.into()] },
            },
        );
        doc
    }

    #[test]
    fn a_node_that_does_not_resolve_leaves_the_tree_incomplete() {
        let pdf = open(&document());
        let tree = NameTree::read(&pdf, b"Dests");
        assert!(!tree.complete);
        let keys: Vec<Vec<u8>> = tree
            .pairs
            .iter()
            .map(|pair| pair.key.clone().into_string().unwrap().to_vec())
            .collect();
        assert_eq!(keys, [b"a".to_vec(), b"b".to_vec()]);
    }

    /// An array shared among many nodes is read once, its pairs taken once.
    #[test]
    fn a_shared_array_is_read_once() {
        let mut doc = Document::new((1, 7));
        with_pages(&mut doc, [Dictionary::new()]);
        let names = doc.add(Value::Array(vec![Value::string("a"), 1.into()]));
        let leaves: Vec<Value> = (0..3)
            .map(|_| doc.add(dict! { "Names" => names }).into())
            .collect();
        let kids = doc.add(Value::Array(leaves));
        let parents: Vec<Value> = (0..3)
            .map(|_| doc.add(dict! { "Kids" => kids }).into())
            .collect();
        catalog(&mut doc).set("Names", dict! { "Dests" => dict! { "Kids" => parents } });
        let pdf = open(&doc);
        let tree = NameTree::read(&pdf, b"Dests");
        assert!(!tree.complete);
        assert_eq!(tree.pairs.len(), 1);
    }

    /// A node reached through two references, one to the other, is read
    /// once.
    #[test]
    fn a_node_named_through_another_reference_is_read_once() {
        let mut doc = Document::new((1, 7));
        with_pages(&mut doc, [Dictionary::new()]);
        let leaf = doc.add(dict! {
            "Names" => vec![Value::string("a"), 1.into()],
        });
        let alias = doc.add(Value::Ref(leaf));
        catalog(&mut doc).set(
            "Names",
            dict! { "Dests" => dict! { "Kids" => vec![leaf.into(), alias.into()] } },
        );
        let pdf = open(&doc);
        let tree = NameTree::read(&pdf, b"Dests");
        assert_eq!(tree.pairs.len(), 1);
    }

    /// A null `/Names`, held in place or referred to, is no entry: the
    /// node's kids are still read.
    #[test]
    fn a_null_names_entry_is_skipped() {
        let mut doc = Document::new((1, 7));
        with_pages(&mut doc, [Dictionary::new()]);
        let leaf = doc.add(dict! {
            "Names" => vec![Value::string("a"), 1.into()],
        });
        let null = doc.add(Value::Null);
        catalog(&mut doc).set(
            "Names",
            dict! {
                "Dests" => dict! { "Names" => null, "Kids" => vec![leaf.into()] },
            },
        );
        let pdf = open(&doc);
        let tree = NameTree::read(&pdf, b"Dests");
        assert!(tree.complete);
        assert_eq!(tree.pairs.len(), 1);
    }

    /// A `/Names` referring to nothing holds no pairs, and the kids are
    /// still read; a null root is no tree, read whole.
    #[test]
    fn a_dangling_names_entry_and_a_null_root_are_absent() {
        let mut doc = Document::new((1, 7));
        with_pages(&mut doc, [Dictionary::new()]);
        let leaf = doc.add(dict! {
            "Names" => vec![Value::string("a"), 1.into()],
        });
        catalog(&mut doc).set(
            "Names",
            dict! {
                "Dests" => dict! {
                    "Names" => Value::Ref(Id(9999)), "Kids" => vec![leaf.into()],
                },
                "EmbeddedFiles" => Value::Null,
            },
        );
        let pdf = open(&doc);
        let tree = NameTree::read(&pdf, b"Dests");
        assert!(tree.complete);
        assert_eq!(tree.pairs.len(), 1);
        let files = NameTree::read(&pdf, b"EmbeddedFiles");
        assert!(files.complete && files.pairs.is_empty());
    }
}
