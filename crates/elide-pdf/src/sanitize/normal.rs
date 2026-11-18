//! The normal form: what every document the rebuild writes holds, and what
//! everything that reads it afterwards may count on.
//!
//! - **References name objects.** A reference names a dictionary or a stream;
//!   a number, name, string or array is held in place, so a value read where
//!   one belongs is the value.
//! - **Fonts and streams are objects.** Every stream, and every font a
//!   resource dictionary names, is an object of its own, so a font, a form or
//!   a glyph procedure is found among the document's objects.
//! - **Content carries its own resources.** Every page, form XObject, tiling
//!   pattern and Type3 font has a `/Resources` dictionary of its own, held in
//!   place with every category of it, so the names its content uses resolve
//!   there and nowhere else.
//! - **A page's content is one stream.**
//! - **Streams are Flate.** A stream is compressed with `/FlateDecode`, or
//!   not at all, with no `/DecodeParms`; only an image stored as a JPEG or
//!   JPEG 2000 file keeps its own encoding, so a stream decodes the same way
//!   everywhere.
//!
//! The rebuild writes this form, the output check verifies it ([`verify`]),
//! and every rebuild the tests make is verified too.

use elide_core::{Error, Result};
use lopdf::{Dictionary, Document, Object, ObjectId};

use super::MAX_NESTING;
use crate::pdf::reader::DictionaryExt;

/// How deep the verifier looks into an object's values held in place: as
/// deep as the rebuild writes them.
const MAX_DEPTH: usize = MAX_NESTING;

/// The most faults one verification names.
const MAX_FAULTS: usize = 8;

/// Verify that `doc` is in the normal form.
///
/// # Errors
///
/// [`ErrorKind::Sanitization`](elide_core::ErrorKind::Sanitization) naming
/// what breaks it.
pub(super) fn verify(doc: &Document) -> Result<()> {
    let mut faults = Faults::default();
    for (&id, object) in &doc.objects {
        faults.object(doc, id, object, 0);
    }
    for (_, value) in doc.trailer.iter() {
        faults.references(doc, None, value, 0);
    }
    for page in doc.get_pages().into_values() {
        let Ok(dict) = doc.get_dictionary(page) else {
            faults.push(format!("page {page:?} is not a dictionary"));
            continue;
        };
        faults.own_resources(dict, &format!("page {page:?}"));
        let one_stream = matches!(
            dict.get(b"Contents"),
            Ok(Object::Reference(id)) if matches!(doc.get_object(*id), Ok(Object::Stream(_)))
        );
        if !one_stream {
            faults.push(format!("page {page:?}'s content is not one stream"));
        }
    }
    faults.into_result()
}

/// What breaks the normal form, as found.
#[derive(Default)]
struct Faults {
    found: Vec<String>,
    count: usize,
}

impl Faults {
    /// A value in the object `id` (or the trailer, for `None`) nests past
    /// [`MAX_DEPTH`], so it cannot be verified.
    fn too_deep(&mut self, id: impl Into<Option<ObjectId>>) {
        match id.into() {
            Some(id) => self.push(format!("{id:?} nests past what is verified")),
            None => self.push("the trailer nests past what is verified".to_owned()),
        }
    }

    fn push(&mut self, fault: String) {
        self.count += 1;
        if self.found.len() < MAX_FAULTS {
            self.found.push(fault);
        }
    }

    /// Check the object `id`, `object`, and every value it holds in place:
    /// its references, and the resources of the content it is or holds.
    fn object(&mut self, doc: &Document, id: ObjectId, object: &Object, depth: usize) {
        if depth > MAX_DEPTH {
            self.too_deep(id);
            return;
        }
        let dict = match object {
            Object::Dictionary(dict) => dict,
            Object::Stream(stream) => {
                if depth > 0 {
                    self.push(format!("a stream is held in place in {id:?}"));
                }
                self.encoding(doc, id, &stream.dict);
                let content = stream.dict.is_subtype(doc, b"Form")
                    || stream
                        .dict
                        .get(b"PatternType")
                        .is_ok_and(|kind| kind.as_i64().ok() == Some(1));
                if content {
                    self.own_resources(&stream.dict, &format!("content stream {id:?}"));
                }
                &stream.dict
            }
            Object::Array(items) => {
                for item in items {
                    self.object(doc, id, item, depth + 1);
                }
                return;
            }
            other => {
                self.references(doc, Some(id), other, depth);
                return;
            }
        };
        if dict.is_subtype(doc, b"Type3") {
            self.own_resources(dict, &format!("Type3 font in {id:?}"));
        }
        for (_, value) in dict.iter() {
            self.object(doc, id, value, depth + 1);
        }
    }

    /// Check that a reference in `value`, held in the object `id` (or the
    /// trailer, for `None`), names a dictionary or a stream.
    fn references(&mut self, doc: &Document, id: Option<ObjectId>, value: &Object, depth: usize) {
        if depth > MAX_DEPTH {
            self.too_deep(id);
            return;
        }
        match value {
            Object::Reference(id) => match doc.objects.get(id) {
                Some(Object::Dictionary(_) | Object::Stream(_)) => {}
                Some(_) => self.push(format!("reference {id:?} names no dictionary or stream")),
                None => self.push(format!("reference {id:?} names nothing")),
            },
            Object::Array(items) => {
                for item in items {
                    self.references(doc, id, item, depth + 1);
                }
            }
            Object::Dictionary(dict) => {
                for (_, value) in dict.iter() {
                    self.references(doc, id, value, depth + 1);
                }
            }
            _ => {}
        }
    }

    /// Check that the stream `id`, of dictionary `dict`, is Flate or not
    /// compressed at all, or is an image stored as a JPEG or JPEG 2000 file.
    fn encoding(&mut self, doc: &Document, id: ObjectId, dict: &Dictionary) {
        let filter = dict.get(b"Filter").ok();
        let flate = match filter {
            None => true,
            Some(Object::Name(name)) => name == b"FlateDecode",
            Some(_) => false,
        };
        let image_file = dict.is_subtype(doc, b"Image")
            && matches!(filter, Some(Object::Name(name)) if name == b"DCTDecode" || name == b"JPXDecode");
        if image_file {
            return;
        }
        if !flate || dict.get(b"DecodeParms").is_ok() {
            self.push(format!("stream {id:?} is not Flate"));
        }
    }

    /// Check that the content `dict`, `what`, has `/Resources` of its own,
    /// and that each font they name is an object of its own.
    fn own_resources(&mut self, dict: &Dictionary, what: &str) {
        let Ok(Object::Dictionary(resources)) = dict.get(b"Resources") else {
            self.push(format!("{what} has no /Resources of its own"));
            return;
        };
        for (category, entries) in resources.iter() {
            if category != b"ProcSet" && !matches!(entries, Object::Dictionary(_)) {
                let category = String::from_utf8_lossy(category);
                self.push(format!(
                    "{what}'s /{category} resources are not held in place"
                ));
            }
        }
        let Ok(Object::Dictionary(fonts)) = resources.get(b"Font") else {
            return;
        };
        for (name, font) in fonts.iter() {
            if !matches!(font, Object::Reference(_)) {
                let name = String::from_utf8_lossy(name);
                self.push(format!("{what}'s font {name} is not an object of its own"));
            }
        }
    }

    fn into_result(self) -> Result<()> {
        if self.count == 0 {
            return Ok(());
        }
        let more = self.count - self.found.len();
        let tail = if more > 0 {
            format!(", and {more} more")
        } else {
            String::new()
        };
        Err(Error::sanitization(format!(
            "PDF is not in the rebuild's normal form: {}{tail}",
            self.found.join("; ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;
    use lopdf::{Stream, dictionary};

    use super::*;
    use crate::fixtures::with_pages;

    /// A one-page document drawing nothing, its page as `page` and with its
    /// own resources unless `page` sets them.
    fn document(page: Dictionary) -> (Document, ObjectId) {
        let mut doc = Document::with_version("1.7");
        let content = doc.add_object(Stream::new(dictionary! {}, Vec::new()));
        let mut dict = dictionary! { "Contents" => content, "Resources" => dictionary! {} };
        dict.extend(&page);
        let id = with_pages(&mut doc, [dict])[0];
        (doc, id)
    }

    fn faulted(doc: &Document) -> bool {
        verify(doc).is_err_and(|e| e.kind() == ErrorKind::Sanitization)
    }

    #[test]
    fn a_page_of_its_own_resources_and_one_stream_is_normal() {
        let (doc, _) = document(Dictionary::new());
        verify(&doc).unwrap();
    }

    /// A reference names an object, never a value held in place.
    #[test]
    fn a_reference_to_a_value_is_not_normal() {
        let (mut doc, page) = document(Dictionary::new());
        let size = doc.add_object(vec![0.into(), 0.into(), 1.into(), 1.into()]);
        doc.get_dictionary_mut(page).unwrap().set("MediaBox", size);
        assert!(faulted(&doc));
    }

    /// Content without resources of its own is not normal, wherever it is.
    #[test]
    fn content_without_its_own_resources_is_not_normal() {
        let (mut doc, page) = document(Dictionary::new());
        let shared = doc.add_object(dictionary! {});
        doc.get_dictionary_mut(page)
            .unwrap()
            .set("Resources", shared);
        assert!(faulted(&doc));

        let (mut doc, _) = document(Dictionary::new());
        doc.add_object(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Form" },
            Vec::new(),
        ));
        assert!(faulted(&doc));

        let (mut doc, _) = document(Dictionary::new());
        doc.add_object(dictionary! {
            "Font" => dictionary! { "T3" => dictionary! { "Type" => "Font", "Subtype" => "Type3" } },
        });
        assert!(faulted(&doc));
    }

    /// A category of resources is held in place, not referred to.
    #[test]
    fn resources_referred_to_are_not_normal() {
        let (mut doc, page) = document(Dictionary::new());
        let images = doc.add_object(dictionary! {});
        doc.get_dictionary_mut(page)
            .unwrap()
            .set("Resources", dictionary! { "XObject" => images });
        assert!(faulted(&doc));
    }

    /// A font a resource dictionary names is an object of its own.
    #[test]
    fn a_font_held_in_place_is_not_normal() {
        let (doc, _) = document(dictionary! {
            "Resources" => dictionary! {
                "Font" => dictionary! { "F1" => dictionary! { "Type" => "Font", "Subtype" => "Type1" } },
            },
        });
        assert!(faulted(&doc));
    }

    /// A stream is Flate with no parameters, unless it is an image file.
    #[test]
    fn a_stream_of_another_encoding_is_not_normal() {
        let (mut doc, _) = document(Dictionary::new());
        doc.add_object(Stream::new(
            dictionary! { "Filter" => "LZWDecode" },
            Vec::new(),
        ));
        assert!(faulted(&doc));

        let (mut doc, _) = document(Dictionary::new());
        doc.add_object(Stream::new(
            dictionary! { "Filter" => "FlateDecode", "DecodeParms" => dictionary! { "Predictor" => 12 } },
            Vec::new(),
        ));
        assert!(faulted(&doc));

        let (mut doc, _) = document(Dictionary::new());
        doc.add_object(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Image", "Filter" => "DCTDecode" },
            Vec::new(),
        ));
        verify(&doc).unwrap();
    }

    /// A value nested past what is verified is a fault, not passed over.
    #[test]
    fn a_value_nested_too_deep_is_not_normal() {
        let (mut doc, _) = document(Dictionary::new());
        let mut value = Object::Reference((9999, 0));
        for _ in 0..=MAX_DEPTH + 1 {
            value = Object::Array(vec![value]);
        }
        doc.add_object(dictionary! { "Deep" => value });
        assert!(faulted(&doc));
    }

    /// A page's content is one stream, not an array of them.
    #[test]
    fn content_in_several_streams_is_not_normal() {
        let (mut doc, page) = document(Dictionary::new());
        let first = doc.add_object(Stream::new(dictionary! {}, Vec::new()));
        let second = doc.add_object(Stream::new(dictionary! {}, Vec::new()));
        doc.get_dictionary_mut(page)
            .unwrap()
            .set("Contents", vec![first.into(), second.into()]);
        assert!(faulted(&doc));
    }
}
