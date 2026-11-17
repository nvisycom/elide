//! Editing name trees in place: dropping pairs from the nodes a
//! [`NameTree`](crate::pdf::reader::NameTree) read.

use lopdf::{Document, Object};

use crate::pdf::reader::DocumentExt;
use crate::pdf::reader::name_tree::{Node, node_dict};

/// Drop from the tree `tree` of `doc`, at each of its `nodes`, every pair
/// `keep` rejects.
pub(crate) fn retain(
    doc: &mut Document,
    tree: &[u8],
    nodes: &[Node],
    keep: impl Fn(&Document, &Object, &Object) -> bool,
) {
    // Decide which pairs each node keeps while the tree is still readable: a
    // value may resolve through other objects, which a mutable borrow of the
    // node would block.
    let kept_by_node: Vec<(Node, Vec<bool>)> = nodes
        .iter()
        .filter_map(|&node| {
            let names = node_dict(doc, tree, node)?.get(b"Names").ok()?;
            let names = doc.resolve_array(names)?;
            let (pairs, _) = names.as_chunks::<2>();
            let kept = pairs
                .iter()
                .map(|[key, value]| keep(doc, key, value))
                .collect();
            Some((node, kept))
        })
        .collect();
    for (node, kept) in kept_by_node {
        let Some(names) = names_mut(doc, tree, node) else {
            continue;
        };
        let mut kept = kept.into_iter();
        let pairs: Vec<Object> = names
            .chunks(2)
            .filter(|pair| pair.len() < 2 || kept.next().unwrap_or(true))
            .flatten()
            .cloned()
            .collect();
        *names = pairs;
    }
}

/// The `/Names` array of the node `node` of the tree `tree` in `doc`,
/// mutably: the node's own, or the array object it refers to.
fn names_mut<'a>(doc: &'a mut Document, tree: &[u8], node: Node) -> Option<&'a mut Vec<Object>> {
    let referenced = node_dict(doc, tree, node)?
        .get(b"Names")
        .and_then(Object::as_reference)
        .ok();
    if let Some(id) = referenced {
        return doc.get_object_mut(id).and_then(Object::as_array_mut).ok();
    }
    let dict = match node {
        Node::Object(id) => doc.get_dictionary_mut(id).ok()?,
        Node::InlineRoot(names) => {
            let names = match names {
                Some(id) => doc.get_dictionary_mut(id).ok()?,
                None => doc
                    .catalog_mut()
                    .ok()?
                    .get_mut(b"Names")
                    .and_then(Object::as_dict_mut)
                    .ok()?,
            };
            names.get_mut(tree).and_then(Object::as_dict_mut).ok()?
        }
    };
    dict.get_mut(b"Names").and_then(Object::as_array_mut).ok()
}

#[cfg(test)]
mod tests {
    use lopdf::{Dictionary, dictionary};

    use super::*;
    use crate::fixtures::with_pages;
    use crate::pdf::reader::NameTree;

    /// A document whose `/Dests` tree has one leaf held in a referenced
    /// `/Names` array.
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
            dictionary! { "Dests" => dictionary! { "Kids" => vec![leaf.into()] } },
        );
        doc
    }

    #[test]
    fn retain_drops_a_pair_from_a_referenced_array() {
        let mut doc = document();
        let nodes = NameTree::read(&doc, b"Dests").nodes;
        retain(&mut doc, b"Dests", &nodes, |_, _, value| {
            value != &Object::Integer(1)
        });
        let pairs: Vec<Object> = NameTree::read(&doc, b"Dests")
            .pairs
            .into_iter()
            .map(|(_, value)| value.clone())
            .collect();
        assert_eq!(pairs, [Object::Integer(2)]);
    }
}
