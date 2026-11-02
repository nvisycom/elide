//! The annotations a sanitized PDF keeps: a link to a place in the same
//! document, reduced to where it is, where it goes, and how it looks. Every
//! other annotation is hidden content, recorded by what it is.

use elide_core::redaction::HiddenContent;
use lopdf::{Document, Object, ObjectId};

use super::ext::SanitizeDictionaryExt;

/// The entries a kept internal link keeps: where it is, where it goes, and how
/// it looks. Its tooltip, author, dates, and name go.
const LINK_KEPT: [&[u8]; 12] = [
    b"Type",
    b"Subtype",
    b"Rect",
    b"Dest",
    b"A",
    b"Border",
    b"BS",
    b"C",
    b"H",
    b"QuadPoints",
    b"F",
    b"P",
];

/// Remove every annotation of `page` but its internal links, returning the
/// hidden content each removed one was.
pub(super) fn keep_internal_links(doc: &mut Document, page: ObjectId) -> Vec<HiddenContent> {
    let annotations: Vec<Object> = match doc.get_dictionary(page).and_then(|p| p.get(b"Annots")) {
        Ok(Object::Reference(id)) => match doc.get_object(*id).and_then(Object::as_array) {
            Ok(array) => array.clone(),
            Err(_) => Vec::new(),
        },
        Ok(Object::Array(array)) => array.clone(),
        _ => return Vec::new(),
    };
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    for annotation in annotations {
        let dict = match &annotation {
            Object::Reference(id) => doc.get_dictionary(*id).ok(),
            Object::Dictionary(dict) => Some(dict),
            _ => None,
        };
        match dict {
            Some(dict) if dict.is_internal_link() => kept.push(annotation),
            Some(dict) => dropped.push(dict.annotation_content()),
            None => dropped.push(HiddenContent::Annotation),
        }
    }
    // A kept link keeps only where it is and where it goes: its tooltip,
    // author, dates, and name are an annotation's text.
    let mut reduced = false;
    for annotation in &mut kept {
        let dict = match annotation {
            Object::Reference(id) => doc
                .get_object_mut(*id)
                .ok()
                .and_then(|o| o.as_dict_mut().ok()),
            Object::Dictionary(dict) => {
                reduced = true;
                Some(dict)
            }
            _ => None,
        };
        if let Some(dict) = dict
            && !dict
                .retain_keys(&LINK_KEPT, |_| Some(HiddenContent::Annotation))
                .is_empty()
        {
            dropped.push(HiddenContent::Annotation);
        }
    }
    if dropped.is_empty() && !reduced {
        return dropped;
    }
    if let Some(page) = doc
        .get_object_mut(page)
        .ok()
        .and_then(|o| o.as_dict_mut().ok())
    {
        if kept.is_empty() {
            page.remove(b"Annots");
        } else {
            page.set("Annots", Object::Array(kept));
        }
    }
    dropped
}
