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
use hayro_syntax::object::{Dict, MaybeRef, Object, Stream};

use super::MAX_NESTING;
use crate::pdf::Pdf;
use crate::pdf::reader::{DictExt, ObjectId, id};

/// How deep the verifier looks into an object's values held in place: as
/// deep as the rebuild writes them.
const MAX_DEPTH: usize = MAX_NESTING;

/// The most faults one verification names.
const MAX_FAULTS: usize = 8;

/// Verify that `pdf` is in the normal form.
///
/// # Errors
///
/// [`ErrorKind::Sanitization`](elide_core::ErrorKind::Sanitization) naming
/// what breaks it.
pub(super) fn verify(pdf: &Pdf) -> Result<()> {
    let mut faults = Faults::default();
    for object in pdf.file().objects() {
        match &object {
            Object::Dict(dict) => {
                if let Some(id) = dict.obj_id() {
                    faults.object(pdf, id, &object, 0);
                }
            }
            Object::Stream(stream) => faults.object(pdf, stream.obj_id(), &object, 0),
            _ => {}
        }
    }
    for page in pdf.pages() {
        let what = format!("page {:?}", page.obj_id());
        faults.own_resources(&page, &what);
        let one_stream = matches!(page.raw(b"Contents"), Some(MaybeRef::Ref(_)))
            && page.get::<Stream<'_>>(b"Contents").is_some();
        if !one_stream {
            faults.push(format!("{what}'s content is not one stream"));
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
    /// A value in the object `id` nests past [`MAX_DEPTH`], so it cannot be
    /// verified.
    fn too_deep(&mut self, id: ObjectId) {
        self.push(format!("{id:?} nests past what is verified"));
    }

    fn push(&mut self, fault: String) {
        self.count += 1;
        if self.found.len() < MAX_FAULTS {
            self.found.push(fault);
        }
    }

    /// Check the object `id`, `object`, and every value it holds in place:
    /// its references, and the resources of the content it is or holds.
    fn object(&mut self, pdf: &Pdf, id: ObjectId, object: &Object<'_>, depth: usize) {
        if depth > MAX_DEPTH {
            self.too_deep(id);
            return;
        }
        let dict = match object {
            Object::Dict(dict) => dict.clone(),
            Object::Stream(stream) => {
                let dict = stream.dict().clone();
                self.encoding(id, &dict);
                let content = dict.is_subtype(b"Form") || dict.number(b"PatternType") == Some(1.0);
                if content {
                    self.own_resources(&dict, &format!("content stream {id:?}"));
                }
                dict
            }
            Object::Array(items) => {
                for item in items.raw_iter() {
                    self.item(pdf, id, &item, depth + 1);
                }
                return;
            }
            _ => return,
        };
        if dict.is_subtype(b"Type3") {
            self.own_resources(&dict, &format!("Type3 font in {id:?}"));
        }
        for (_, value) in dict.entries() {
            self.item(pdf, id, &value, depth + 1);
        }
    }

    /// Check the value `item` held in the object `owner`: a reference names
    /// a dictionary or a stream; a value held in place is checked as one.
    fn item(&mut self, pdf: &Pdf, owner: ObjectId, item: &MaybeRef<Object<'_>>, depth: usize) {
        match item {
            MaybeRef::Ref(reference) => {
                let target = id(*reference);
                match pdf.object(target) {
                    Some(Object::Dict(_) | Object::Stream(_)) => {}
                    Some(_) => self.push(format!(
                        "reference {target:?} names no dictionary or stream"
                    )),
                    None => self.push(format!("reference {target:?} names nothing")),
                }
            }
            MaybeRef::NotRef(value) => self.object(pdf, owner, value, depth),
        }
    }

    /// Check that the stream `id`, of dictionary `dict`, is Flate or not
    /// compressed at all, or is an image stored as a JPEG or JPEG 2000 file.
    fn encoding(&mut self, id: ObjectId, dict: &Dict<'_>) {
        let filter = dict.get::<Object<'_>>(b"Filter");
        let name = filter.clone().and_then(Object::into_name);
        let flate = match (&filter, &name) {
            (None, _) => true,
            (Some(_), Some(name)) => &**name == b"FlateDecode",
            (Some(_), None) => false,
        };
        let image_file = dict.is_subtype(b"Image")
            && name.is_some_and(|name| &*name == b"DCTDecode" || &*name == b"JPXDecode");
        if image_file {
            return;
        }
        if !flate || dict.contains_key(b"DecodeParms") {
            self.push(format!("stream {id:?} is not Flate"));
        }
    }

    /// Check that the content `dict`, `what`, has `/Resources` of its own,
    /// and that each font they name is an object of its own.
    fn own_resources(&mut self, dict: &Dict<'_>, what: &str) {
        let Some(MaybeRef::NotRef(Object::Dict(resources))) = dict.raw(b"Resources") else {
            self.push(format!("{what} has no /Resources of its own"));
            return;
        };
        for (category, entries) in resources.entries() {
            if &*category != b"ProcSet" && !matches!(entries, MaybeRef::NotRef(Object::Dict(_))) {
                let category = String::from_utf8_lossy(&category);
                self.push(format!(
                    "{what}'s /{category} resources are not held in place"
                ));
            }
        }
        let Some(MaybeRef::NotRef(Object::Dict(fonts))) = resources.raw(b"Font") else {
            return;
        };
        for (name, font) in fonts.entries() {
            if !matches!(font, MaybeRef::Ref(_)) {
                let name = String::from_utf8_lossy(&name);
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

    use super::*;
    use crate::fixtures::{held, with_pages};
    use crate::pdf::object::{Dictionary, Id, Stream, Value, dict};
    use crate::pdf::writer::Document;

    /// A one-page document drawing nothing, its page as `page` and with its
    /// own resources unless `page` sets them.
    fn document(page: Dictionary) -> (Document, Id) {
        let mut doc = Document::new((1, 7));
        let content = doc.add(Stream::new(dict! {}, Vec::new()));
        let mut dict = dict! { "Contents" => content, "Resources" => dict! {} };
        for (key, value) in page.iter() {
            dict.set(key, value.clone());
        }
        let id = with_pages(&mut doc, [dict])[0];
        (doc, id)
    }

    /// The dictionary of the page `page` of `doc`, to change.
    fn page_mut(doc: &mut Document, page: Id) -> &mut Dictionary {
        doc.get_mut(page).and_then(Value::as_dict_mut).unwrap()
    }

    /// Whether `doc`, written with `objects` kept, breaks the normal form.
    fn faulted(doc: Document, objects: &[Id]) -> bool {
        verify(&held(doc, objects)).is_err_and(|e| e.kind() == ErrorKind::Sanitization)
    }

    #[test]
    fn a_page_of_its_own_resources_and_one_stream_is_normal() {
        let (doc, _) = document(Dictionary::new());
        verify(&held(doc, &[])).unwrap();
    }

    /// A reference names an object, never a value held in place.
    #[test]
    fn a_reference_to_a_value_is_not_normal() {
        let (mut doc, page) = document(Dictionary::new());
        let size = doc.add(vec![0.into(), 0.into(), 1.into(), 1.into()]);
        page_mut(&mut doc, page).set("MediaBox", size);
        assert!(faulted(doc, &[]));
    }

    /// Content without resources of its own is not normal, wherever it is.
    #[test]
    fn content_without_its_own_resources_is_not_normal() {
        let (mut doc, page) = document(Dictionary::new());
        let shared = doc.add(dict! {});
        page_mut(&mut doc, page).set("Resources", shared);
        assert!(faulted(doc, &[]));

        let (mut doc, _) = document(Dictionary::new());
        let form = doc.add(Stream::new(
            dict! { "Type" => "XObject", "Subtype" => "Form" },
            Vec::new(),
        ));
        assert!(faulted(doc, &[form]));

        let (mut doc, _) = document(Dictionary::new());
        let holder = doc.add(dict! {
            "Font" => dict! { "T3" => dict! { "Type" => "Font", "Subtype" => "Type3" } },
        });
        assert!(faulted(doc, &[holder]));
    }

    /// A category of resources is held in place, not referred to.
    #[test]
    fn resources_referred_to_are_not_normal() {
        let (mut doc, page) = document(Dictionary::new());
        let images = doc.add(dict! {});
        page_mut(&mut doc, page).set("Resources", dict! { "XObject" => images });
        assert!(faulted(doc, &[]));
    }

    /// A font a resource dictionary names is an object of its own.
    #[test]
    fn a_font_held_in_place_is_not_normal() {
        let (doc, _) = document(dict! {
            "Resources" => dict! {
                "Font" => dict! { "F1" => dict! { "Type" => "Font", "Subtype" => "Type1" } },
            },
        });
        assert!(faulted(doc, &[]));
    }

    /// A stream is Flate with no parameters, unless it is an image file.
    #[test]
    fn a_stream_of_another_encoding_is_not_normal() {
        let (mut doc, _) = document(Dictionary::new());
        let lzw = doc.add(Stream::new(dict! { "Filter" => "LZWDecode" }, Vec::new()));
        assert!(faulted(doc, &[lzw]));

        let (mut doc, _) = document(Dictionary::new());
        let predicted = doc.add(Stream::new(
            dict! { "Filter" => "FlateDecode", "DecodeParms" => dict! { "Predictor" => 12 } },
            Vec::new(),
        ));
        assert!(faulted(doc, &[predicted]));

        let (mut doc, _) = document(Dictionary::new());
        let jpeg = doc.add(Stream::new(
            dict! { "Type" => "XObject", "Subtype" => "Image", "Filter" => "DCTDecode" },
            Vec::new(),
        ));
        verify(&held(doc, &[jpeg])).unwrap();
    }

    /// A value nested past what is verified is a fault, not passed over: a
    /// leaf as deep as is verified is normal, one level deeper is not.
    #[test]
    fn a_value_nested_too_deep_is_not_normal() {
        let nested = |levels: usize| {
            let (mut doc, _) = document(Dictionary::new());
            let mut value = Value::Int(0);
            for _ in 0..levels {
                value = Value::Array(vec![value]);
            }
            let deep = doc.add(dict! { "Deep" => value });
            (doc, deep)
        };
        let (doc, deep) = nested(MAX_DEPTH - 1);
        verify(&held(doc, &[deep])).unwrap();
        let (doc, deep) = nested(MAX_DEPTH);
        assert!(faulted(doc, &[deep]));
    }

    /// A page's content is one stream, not an array of them.
    #[test]
    fn content_in_several_streams_is_not_normal() {
        let (mut doc, page) = document(Dictionary::new());
        let first = doc.add(Stream::new(dict! {}, Vec::new()));
        let second = doc.add(Stream::new(dict! {}, Vec::new()));
        page_mut(&mut doc, page).set("Contents", vec![first.into(), second.into()]);
        assert!(faulted(doc, &[]));
    }
}
