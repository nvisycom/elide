//! Direct values for the keys that say what an object is.
//!
//! Any value in a PDF may be stored as a reference to an object holding it:
//! `/Subtype 7 0 R` with object 7 the name `/Form`. A check that reads such a
//! key directly would misclassify the object, and a form whose text is never
//! walked, or a pattern never inspected, ships unredacted. Every key below is
//! resolved in place at decode, so everything after reads a direct value.

use lopdf::{Dictionary, Document, Object, ObjectId};

/// The keys every later read classifies or measures an object by.
const CLASSIFYING_KEYS: [&[u8]; 12] = [
    b"Type",
    b"Subtype",
    b"PatternType",
    b"Rotate",
    b"UserUnit",
    b"MediaBox",
    b"CropBox",
    b"Filter",
    b"Width",
    b"Height",
    b"BitsPerComponent",
    b"ImageMask",
];

/// How deep a value nests dictionaries and arrays before this stops looking.
const MAX_DEPTH: usize = 32;

/// How many references a value is followed through before it is taken as is.
const MAX_HOPS: usize = 16;

/// Resolve every classifying key of every dictionary in `doc`, nested ones
/// included, to a direct value, and each element of an array it holds.
pub(super) fn resolve(doc: &mut Document) {
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    for id in ids {
        let Some(mut object) = doc.objects.remove(&id) else {
            continue;
        };
        visit(doc, &mut object, 0);
        doc.objects.insert(id, object);
    }
}

/// Resolve the classifying keys of `object` and everything it nests.
fn visit(doc: &Document, object: &mut Object, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    match object {
        Object::Dictionary(dict) => resolve_dict(doc, dict, depth),
        Object::Stream(stream) => resolve_dict(doc, &mut stream.dict, depth),
        Object::Array(items) => {
            for item in items {
                visit(doc, item, depth + 1);
            }
        }
        _ => {}
    }
}

fn resolve_dict(doc: &Document, dict: &mut Dictionary, depth: usize) {
    for key in CLASSIFYING_KEYS {
        let Ok(value) = dict.get_mut(key) else {
            continue;
        };
        if let Some(direct) = direct(doc, value) {
            *value = direct;
        }
        if let Object::Array(items) = value {
            for item in items.iter_mut() {
                if let Some(direct) = direct(doc, item) {
                    *item = direct;
                }
            }
        }
    }
    for (_, value) in dict.iter_mut() {
        visit(doc, value, depth + 1);
    }
}

/// The value `object` refers to, when it is a reference to a name, number,
/// boolean, or array: a dictionary or stream is an object of its own and is
/// left referenced.
fn direct(doc: &Document, object: &Object) -> Option<Object> {
    let &Object::Reference(mut id) = object else {
        return None;
    };
    for _ in 0..MAX_HOPS {
        match doc.objects.get(&id)? {
            Object::Reference(next) => id = *next,
            Object::Dictionary(_) | Object::Stream(_) => return None,
            value => return Some(value.clone()),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use lopdf::dictionary;

    use super::*;

    #[test]
    fn a_referenced_classifier_becomes_direct() {
        let mut doc = Document::with_version("1.7");
        let form = doc.add_object(Object::Name(b"Form".to_vec()));
        let box_edge = doc.add_object(Object::Integer(612));
        let id = doc.add_object(dictionary! {
            "Subtype" => form,
            "MediaBox" => vec![0.into(), 0.into(), box_edge.into(), 792.into()],
            "Resources" => dictionary! { "Inner" => dictionary! { "Type" => form } },
        });
        resolve(&mut doc);
        let dict = doc.get_dictionary(id).unwrap();
        assert_eq!(
            dict.get(b"Subtype").unwrap(),
            &Object::Name(b"Form".to_vec())
        );
        assert_eq!(
            dict.get(b"MediaBox").unwrap().as_array().unwrap()[2],
            Object::Integer(612)
        );
        let inner = dict
            .get(b"Resources")
            .and_then(Object::as_dict)
            .and_then(|r| r.get(b"Inner"))
            .and_then(Object::as_dict)
            .unwrap();
        assert_eq!(inner.get(b"Type").unwrap(), &Object::Name(b"Form".to_vec()));
    }

    #[test]
    fn a_referenced_dictionary_stays_referenced() {
        let mut doc = Document::with_version("1.7");
        let target = doc.add_object(dictionary! { "Type" => "Font" });
        let id = doc.add_object(dictionary! { "Type" => target });
        resolve(&mut doc);
        assert_eq!(
            doc.get_dictionary(id).unwrap().get(b"Type").unwrap(),
            &Object::Reference(target)
        );
    }
}
