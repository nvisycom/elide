//! Strip the document structures that retain copies of text after a text
//! redaction: annotations (their contents and URIs), the `/Info` dictionary,
//! the `/Metadata` stream, interactive form values (`/AcroForm` + XFA),
//! embedded file attachments (`/Names`), the outline (`/Outlines` bookmark
//! titles), and document-level actions (`/OpenAction`, `/AA`).
//!
//! Removing the reference is not enough, an orphaned object is still written by
//! `save_to`, so the text survives in the bytes (a real leak). Each stripped
//! structure's whole subtree of referenced objects is therefore deleted from the
//! document (recursively), except objects still shared with surviving content,
//! which are kept.

use std::collections::BTreeSet;

use lopdf::{Document, Object, ObjectId};

use super::graph::{collect_entry, collect_owned, referenced_from_survivors, resolve_array};

/// Remove annotations, `/Info`, and `/Metadata`, deleting their objects.
pub(super) fn sanitize(doc: &mut Document) {
    let mut doomed: BTreeSet<ObjectId> = BTreeSet::new();

    // Page annotations: delete every object reachable from each page's
    // `/Annots`, the array object itself (when `/Annots` is a reference) and
    // the annotation dictionaries it lists (whether inline or referenced), then
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
            // the annotations, delete it and everything it owns.
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
        // Page thumbnail (`/Thumb`): a small rendered preview of the page, which
        // reproduces the redacted page pixels. Delete its object subtree and drop
        // the key.
        if let Ok(thumb) = doc
            .get_object(page_id)
            .and_then(Object::as_dict)
            .and_then(|d| d.get(b"Thumb"))
            && let Ok(thumb_id) = thumb.as_reference()
        {
            collect_owned(doc, thumb_id, &mut doomed);
        }
        if let Ok(dict) = doc.get_object_mut(page_id).and_then(Object::as_dict_mut) {
            dict.remove(b"Annots");
            dict.remove(b"Thumb");
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

    // Optional-content (layers): remove the catalog's `/OCProperties`
    // configuration and clear every object's `/OC` membership mark, so no
    // content is gated behind a hidden layer and no layer config survives.
    strip_optional_content(doc, &mut doomed);

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

/// Remove optional-content (layer) machinery: the catalog's `/OCProperties`
/// configuration and every `/OC` membership mark.
///
/// Content in a hidden layer is still in the file; a viewer that turns the layer
/// on reveals it. Clearing the `/OC` marks first detaches all content from its
/// layers (so nothing stays hidden and no surviving object references an OCG),
/// then the `/OCProperties` subtree is doomed. Because the marks are already
/// gone, the survivor guard no longer keeps the OCG dictionaries, so the whole
/// layer configuration is pruned.
fn strip_optional_content(doc: &mut Document, doomed: &mut BTreeSet<ObjectId>) {
    // A resource dictionary's `/Properties` holds the marked-content property
    // lists a `/OC /MC0 BDC ... EMC` sequence resolves `/MC0` through (OCGs or
    // OCMDs). Find every resource dictionary (reached via a `/Resources` key,
    // inline or referenced) so its `/Properties` can be cleared, detaching those
    // OCGs. `/Properties` is scoped to resource dictionaries here rather than
    // stripped from every dictionary, since the key is meaningful elsewhere.
    let mut resource_dict_ids: BTreeSet<ObjectId> = BTreeSet::new();
    for object in doc.objects.values() {
        let dict = match object {
            Object::Dictionary(dict) => dict,
            Object::Stream(stream) => &stream.dict,
            _ => continue,
        };
        if let Ok(resources) = dict.get(b"Resources")
            && let Ok(id) = resources.as_reference()
        {
            resource_dict_ids.insert(id);
        }
    }

    // Detach every object from its layer: drop the `/OC` membership mark on any
    // dictionary or stream dict, and clear an inline `/Resources`' `/Properties`.
    for object in doc.objects.values_mut() {
        let dict = match object {
            Object::Dictionary(dict) => dict,
            Object::Stream(stream) => &mut stream.dict,
            _ => continue,
        };
        dict.remove(b"OC");
        if let Ok(Object::Dictionary(resources)) = dict.get_mut(b"Resources") {
            resources.remove(b"Properties");
        }
    }
    // Clear `/Properties` on each standalone (referenced) resource dictionary.
    for id in resource_dict_ids {
        if let Some(Object::Dictionary(dict)) = doc.objects.get_mut(&id) {
            dict.remove(b"Properties");
        }
    }

    // Doom the catalog's optional-content configuration and drop the key.
    let oc = doc
        .catalog()
        .and_then(|c| c.get(b"OCProperties"))
        .ok()
        .cloned();
    if let Some(oc) = oc {
        collect_entry(doc, &oc, doomed);
    }
    if let Ok(catalog) = doc.catalog_mut() {
        catalog.remove(b"OCProperties");
    }
}
