//! [`ObjectRole`]: the neutral classification of a PDF object the redaction
//! strategies act on.
//!
//! An object-editing strategy (image replacement, page reflatten, sanitize) must
//! never clobber a structural object, the catalog, the page tree, a page, since
//! doing so corrupts the document rather than redacting it. Rather than each
//! strategy hand-inspecting `/Type`/`/Subtype`, they ask an object's
//! [`ObjectRole`] and consult [`is_protected`](ObjectRole::is_protected)
//! / [`is_whole_object_replaceable`](ObjectRole::is_whole_object_replaceable).
//! This is the PDF analogue of the OOXML `PartRole` seam.

use lopdf::{Document, Object, ObjectId};

/// What a PDF object is, for the purpose of redaction guards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ObjectRole {
    /// An image XObject (`/Subtype /Image`): pixel content that a redaction may
    /// replace wholesale with a redacted image.
    ImageXObject,
    /// A structural object that defines the document's shape, identified by its
    /// `/Type`: the catalog, the page tree (`/Pages`), or a page. Never replaced
    /// or deleted wholesale, that would corrupt the document. (A resource
    /// dictionary is also structural, but carries no `/Type` and is only
    /// recognisable through the `/Resources` entry that references it, so it is
    /// not classified from the object alone here.)
    Structure,
    /// Anything else (a content stream, a font, an annotation, a plain
    /// dictionary): not classified more finely here.
    Other,
}

impl ObjectRole {
    /// Classify the object `id` names in `doc`.
    #[must_use]
    pub fn of(doc: &Document, id: ObjectId) -> Self {
        let Ok(object) = doc.get_object(id) else {
            return Self::Other;
        };
        let dict = match object {
            Object::Dictionary(dict) => dict,
            Object::Stream(stream) => &stream.dict,
            _ => return Self::Other,
        };
        let type_name = dict.get(b"Type").and_then(Object::as_name).ok();
        match type_name {
            Some(b"Catalog") | Some(b"Pages") | Some(b"Page") => return Self::Structure,
            _ => {}
        }
        if dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image".as_slice()) {
            return Self::ImageXObject;
        }
        Self::Other
    }

    /// Whether this object defines document structure and so must never be
    /// deleted or replaced wholesale.
    #[must_use]
    pub fn is_protected(self) -> bool {
        matches!(self, Self::Structure)
    }

    /// Whether an object of this role may be replaced wholesale with new bytes
    /// (an image XObject swapped for a redacted image). Structural objects never
    /// may; only their referenced content is edited.
    #[must_use]
    pub fn is_whole_object_replaceable(self) -> bool {
        matches!(self, Self::ImageXObject)
    }
}

#[cfg(test)]
mod tests {
    use lopdf::{Stream, dictionary};

    use super::*;

    #[test]
    fn classifies_and_guards_the_object_roles() {
        let mut doc = Document::with_version("1.5");
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog" });
        let pages = doc.add_object(dictionary! { "Type" => "Pages" });
        let page = doc.add_object(dictionary! { "Type" => "Page" });
        let image = doc.add_object(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Image" },
            b"pixels".to_vec(),
        ));
        let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1" });

        // Structural objects are protected and never wholesale-replaceable.
        for id in [catalog, pages, page] {
            let role = ObjectRole::of(&doc, id);
            assert_eq!(role, ObjectRole::Structure);
            assert!(role.is_protected());
            assert!(!role.is_whole_object_replaceable());
        }
        // An image XObject is the one replaceable role, and not protected.
        let image_role = ObjectRole::of(&doc, image);
        assert_eq!(image_role, ObjectRole::ImageXObject);
        assert!(image_role.is_whole_object_replaceable());
        assert!(!image_role.is_protected());
        // A font is neither protected nor replaceable.
        let font_role = ObjectRole::of(&doc, font);
        assert_eq!(font_role, ObjectRole::Other);
        assert!(!font_role.is_protected());
        assert!(!font_role.is_whole_object_replaceable());
    }
}
