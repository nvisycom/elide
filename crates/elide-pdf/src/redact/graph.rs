//! Object-graph reachability: the shared primitives the object-editing
//! redaction strategies use to delete a subtree without orphaning shared objects.
//!
//! A strategy marks the objects it wants gone (`collect_owned` /
//! `collect_entry`), then [`referenced_from_survivors`] returns which of those
//! are still reachable from surviving content and so must be kept. Removing only
//! the truly-unreachable ones prunes a stripped structure's whole subtree while
//! leaving no dangling reference. Used by `sanitize`, `redact_images`, and
//! `redact_pages`.

use std::collections::BTreeSet;

use lopdf::{Document, Object, ObjectId};

/// The doomed ids still reachable from surviving content, which must therefore
/// be kept rather than deleted.
///
/// A doomed object referenced by an object *not* in `doomed` (or by the trailer,
/// already cleared of the stripped keys) is shared with surviving content; and so
/// is every doomed object reachable *from* such an object, the full closure, so
/// keeping a shared object never leaves it with a dangling reference to a doomed
/// descendant that was removed.
pub(super) fn referenced_from_survivors(
    doc: &Document,
    doomed: &BTreeSet<ObjectId>,
) -> BTreeSet<ObjectId> {
    // Seed with the doomed objects directly referenced by any survivor or the
    // trailer, then walk the closure: every doomed object reachable from a kept
    // one is also shared (a kept parent still points at it), so it must be kept
    // too, or removing it would leave that parent with a dangling reference.
    let mut kept = BTreeSet::new();
    let mut stack: Vec<ObjectId> = Vec::new();

    let survivor_refs = doc
        .objects
        .iter()
        .filter(|(id, _)| !doomed.contains(id))
        .flat_map(|(_, object)| object_references(object))
        .chain(dict_references(&doc.trailer));
    for referenced in survivor_refs {
        if doomed.contains(&referenced) && kept.insert(referenced) {
            stack.push(referenced);
        }
    }

    while let Some(id) = stack.pop() {
        let Ok(object) = doc.get_object(id) else {
            continue;
        };
        for referenced in object_references(object) {
            if doomed.contains(&referenced) && kept.insert(referenced) {
                stack.push(referenced);
            }
        }
    }
    kept
}

/// Mark `id` and the whole subtree of objects it references for deletion,
/// recursively. A shared object that also belongs to surviving content is
/// pruned back later (see [`referenced_from_survivors`]), so this can gather the
/// full subtree without fear of collateral damage, e.g. an annotation's
/// appearance stream and everything *it* references, at any depth.
pub(super) fn collect_owned(doc: &Document, id: ObjectId, doomed: &mut BTreeSet<ObjectId>) {
    let mut worklist = vec![id];
    while let Some(id) = worklist.pop() {
        if !doomed.insert(id) {
            continue;
        }
        if let Ok(object) = doc.get_object(id) {
            worklist.extend(object_references(object));
        }
    }
}

/// Mark a catalog entry's owned subtree for deletion. An indirect reference
/// dooms the referenced object and everything under it; an inline *dictionary*
/// is removed with its catalog key, so only the objects it references are doomed
/// (recursively).
///
/// An inline array is deliberately *not* treated as an owned subtree: a
/// destination array such as `/OpenAction [page /FitH]` references a live page,
/// not owned content, and following it would doom the document. The
/// survivor-reference guard would not recover it, since the whole page tree can
/// become doomed transitively (a page's `/Parent` back-pointer included).
pub(super) fn collect_entry(doc: &Document, entry: &Object, doomed: &mut BTreeSet<ObjectId>) {
    match entry {
        Object::Reference(id) => collect_owned(doc, *id, doomed),
        Object::Dictionary(dict) => {
            for id in dict_references(dict) {
                collect_owned(doc, id, doomed);
            }
        }
        _ => {}
    }
}

/// Resolve an object that is (a reference to) an array of references into the
/// referenced object ids.
pub(super) fn resolve_array(doc: &Document, object: &Object) -> Option<Vec<ObjectId>> {
    let array = match object {
        Object::Reference(id) => doc.get_object(*id).ok()?.as_array().ok()?,
        Object::Array(a) => a,
        _ => return None,
    };
    Some(array.iter().filter_map(|o| o.as_reference().ok()).collect())
}

/// Every object id an object (dict, stream, or array) references, at any depth
/// within its own inline structure.
fn object_references(object: &Object) -> Vec<ObjectId> {
    match object {
        Object::Dictionary(d) => dict_references(d),
        Object::Stream(s) => dict_references(&s.dict),
        Object::Array(a) => a.iter().flat_map(referenced_ids).collect(),
        _ => Vec::new(),
    }
}

/// Every object id referenced by a dictionary's values.
fn dict_references(dict: &lopdf::Dictionary) -> Vec<ObjectId> {
    dict.iter().flat_map(|(_, v)| referenced_ids(v)).collect()
}

/// The object ids directly referenced by `value` (through a reference, an
/// array, or a nested dictionary).
fn referenced_ids(value: &Object) -> Vec<ObjectId> {
    match value {
        Object::Reference(id) => vec![*id],
        Object::Array(a) => a.iter().flat_map(referenced_ids).collect(),
        Object::Dictionary(d) => d.iter().flat_map(|(_, v)| referenced_ids(v)).collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use lopdf::dictionary;

    use super::*;

    /// A dictionary object holding a single `/Kid` reference to `child`.
    fn parent_of(child: ObjectId) -> Object {
        Object::Dictionary(dictionary! { "Kid" => Object::Reference(child) })
    }

    #[test]
    fn keeps_the_whole_shared_closure_not_just_the_first_hop() {
        // survivor -> a -> b, with both a and b doomed. Keeping only the
        // directly-referenced `a` while deleting `b` would leave `a` with a
        // dangling reference; the closure must keep both.
        let mut doc = Document::with_version("1.5");
        let b = doc.add_object(Object::Dictionary(dictionary! {}));
        let a = doc.add_object(parent_of(b));
        let survivor = doc.add_object(parent_of(a));

        let doomed: BTreeSet<ObjectId> = [a, b].into_iter().collect();
        // `survivor` is not doomed, so it anchors the closure.
        assert!(!doomed.contains(&survivor));

        let kept = referenced_from_survivors(&doc, &doomed);
        assert!(kept.contains(&a), "the directly-referenced object is kept");
        assert!(
            kept.contains(&b),
            "a doomed object reachable only through another kept object must \
             also be kept, or the retained parent dangles"
        );
    }

    #[test]
    fn a_doomed_object_no_survivor_references_is_not_kept() {
        // survivor -> a; b hangs off a but nothing outside `doomed` reaches it,
        // and a is doomed, so the closure keeps a (shared) but drops b.
        let mut doc = Document::with_version("1.5");
        let unreferenced = doc.add_object(Object::Dictionary(dictionary! {}));
        let a = doc.add_object(Object::Dictionary(dictionary! {}));
        let _survivor = doc.add_object(parent_of(a));

        let doomed: BTreeSet<ObjectId> = [a, unreferenced].into_iter().collect();
        let kept = referenced_from_survivors(&doc, &doomed);
        assert!(kept.contains(&a));
        assert!(
            !kept.contains(&unreferenced),
            "a doomed object no survivor reaches is removed"
        );
    }
}
