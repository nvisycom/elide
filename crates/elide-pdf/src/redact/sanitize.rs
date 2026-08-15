//! Strip the document structures that retain copies of text after a text
//! redaction: annotations (their contents and URIs), the `/Info` dictionary,
//! the `/Metadata` stream, interactive form values (`/AcroForm` + XFA),
//! embedded file attachments (`/Names`), the outline (`/Outlines` bookmark
//! titles), and document-level actions (`/OpenAction`, `/AA`).
//!
//! Removing the reference is not enough — an orphaned object is still written by
//! `save_to`, so the text survives in the bytes (a real leak). Each stripped
//! structure's whole subtree of referenced objects is therefore deleted from the
//! document (recursively), except objects still shared with surviving content,
//! which are kept.

use std::collections::BTreeSet;

use lopdf::{Document, Object, ObjectId};

/// Remove annotations, `/Info`, and `/Metadata`, deleting their objects.
pub(super) fn sanitize(doc: &mut Document) {
    let mut doomed: BTreeSet<ObjectId> = BTreeSet::new();

    // Page annotations: delete every object reachable from each page's
    // `/Annots` — the array object itself (when `/Annots` is a reference) and
    // the annotation dictionaries it lists (whether inline or referenced) — then
    // drop the key. The array object can hold the annotation dicts inline, so it
    // must be deleted too, not just the dicts it references.
    let page_ids: Vec<ObjectId> = doc.get_pages().values().copied().collect();
    for page_id in page_ids {
        if let Ok(annots) = doc
            .get_object(page_id)
            .and_then(Object::as_dict)
            .and_then(|d| d.get(b"Annots"))
        {
            // If `/Annots` is a reference to an array object, that object holds
            // the annotations — delete it and everything it owns.
            if let Ok(array_id) = annots.as_reference() {
                collect_owned(doc, array_id, &mut doomed);
            }
            // Also delete each annotation the array references (referenced form).
            if let Some(refs) = resolve_array(doc, annots) {
                for id in refs {
                    collect_owned(doc, id, &mut doomed);
                }
            }
        }
        if let Ok(dict) = doc.get_object_mut(page_id).and_then(Object::as_dict_mut) {
            dict.remove(b"Annots");
        }
    }

    // Document information dictionary (`/Info` in the trailer).
    if let Ok(info_id) = doc.trailer.get(b"Info").and_then(Object::as_reference) {
        collect_owned(doc, info_id, &mut doomed);
    }
    doc.trailer.remove(b"Info");

    // Metadata stream (`/Metadata` in the catalog).
    let meta = doc.catalog().and_then(|c| c.get(b"Metadata")).ok().cloned();
    if let Some(meta) = meta {
        collect_entry(doc, &meta, &mut doomed);
    }
    if let Ok(catalog) = doc.catalog_mut() {
        catalog.remove(b"Metadata");
    }

    // Catalog structures that carry copies of text outside the page content:
    // interactive form field values (`/AcroForm`, incl. XFA), embedded file
    // attachments (`/Names /EmbeddedFiles`), the document outline
    // (`/Outlines`, bookmark titles), and document-level scripts / open actions.
    for key in [
        b"AcroForm".as_slice(),
        b"Outlines",
        b"Names",
        b"OpenAction",
        b"AA",
    ] {
        let entry = doc.catalog().and_then(|c| c.get(key)).ok().cloned();
        if let Some(entry) = entry {
            collect_entry(doc, &entry, &mut doomed);
        }
        if let Ok(catalog) = doc.catalog_mut() {
            catalog.remove(key);
        }
    }

    // A doomed object that is still referenced from a *surviving* object is
    // shared (e.g. a font or resource the page content also uses) and must not
    // be deleted. Keep only the objects unreferenced from outside the doomed
    // set, so the whole stripped subtree goes without collateral damage.
    let referenced_by_survivors = referenced_from_survivors(doc, &doomed);
    for id in &doomed {
        if !referenced_by_survivors.contains(id) {
            doc.objects.remove(id);
        }
    }
}

/// The doomed ids that are still referenced by an object *not* in `doomed`
/// (nor the trailer, already cleared of the stripped keys). Such an object is
/// shared with surviving content and must be kept.
pub(super) fn referenced_from_survivors(
    doc: &Document,
    doomed: &BTreeSet<ObjectId>,
) -> BTreeSet<ObjectId> {
    let mut kept = BTreeSet::new();
    for (id, object) in &doc.objects {
        if doomed.contains(id) {
            continue;
        }
        for referenced in object_references(object) {
            if doomed.contains(&referenced) {
                kept.insert(referenced);
            }
        }
    }
    // Also treat trailer references (e.g. `/Root`) as surviving.
    for referenced in dict_references(&doc.trailer) {
        if doomed.contains(&referenced) {
            kept.insert(referenced);
        }
    }
    kept
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

/// Resolve an object that is (a reference to) an array of references into the
/// referenced object ids.
fn resolve_array(doc: &Document, object: &Object) -> Option<Vec<ObjectId>> {
    let array = match object {
        Object::Reference(id) => doc.get_object(*id).ok()?.as_array().ok()?,
        Object::Array(a) => a,
        _ => return None,
    };
    Some(array.iter().filter_map(|o| o.as_reference().ok()).collect())
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
fn collect_entry(doc: &Document, entry: &Object, doomed: &mut BTreeSet<ObjectId>) {
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

/// Mark `id` and the whole subtree of objects it references for deletion,
/// recursively. A shared object that also belongs to surviving content is
/// pruned back later (see [`referenced_from_survivors`]), so this can gather the
/// full subtree without fear of collateral damage — e.g. an annotation's
/// appearance stream and everything *it* references, at any depth.
fn collect_owned(doc: &Document, id: ObjectId, doomed: &mut BTreeSet<ObjectId>) {
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
