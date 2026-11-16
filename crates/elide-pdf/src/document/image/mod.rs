//! A PDF's embedded images: how one is identified and encoded, how it reaches
//! the image pipeline, and how a redacted or unreadable one goes back.
//!
//! Every image XObject surfaces as the file the pipeline reads, wherever it is
//! drawn from ([`surfaced`]): a JPEG or JPEG 2000 image as it is stored, raw
//! samples decoded to a PNG ([`samples`]). One that cannot surface (fax or
//! JBIG2 samples, a JPEG under a further filter, samples of an unsupported
//! kind, a stencil mask, an inline image) is removed at decode instead
//! ([`remove`](fn@remove)).
//! A redacted image is written back over its XObject
//! ([`replace`](fn@replace)).

mod remove;
mod replace;
mod samples;

use bytes::Bytes;
use elide_core::{Error, Result};
use lopdf::{Document, Object, ObjectId, Stream};

pub(crate) use self::remove::remove;
pub(crate) use self::replace::{ImageReplacement, replace};
use crate::document::object::DictionaryExt;

/// The identity of an image XObject within a PDF: its indirect-object id
/// (object number and generation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ImageId {
    /// The indirect object number.
    pub(crate) number: u32,
    /// The object generation.
    pub(crate) generation: u16,
}

impl ImageId {
    /// The image at object `(number, generation)`.
    pub(crate) fn new(number: u32, generation: u16) -> Self {
        Self { number, generation }
    }

    /// This id as a lopdf `ObjectId` pair.
    pub(crate) fn object(self) -> ObjectId {
        (self.number, self.generation)
    }
}

/// The encoding of an embedded image, from its PDF stream filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmbeddingKind {
    /// A JPEG image (`DCTDecode` filter).
    Jpeg,
    /// A JPEG 2000 image (`JPXDecode` filter).
    Jpeg2000,
    /// A CCITT-fax or JBIG2 image, or a JPEG under a further filter, which no
    /// image codec reads as stored.
    Unreadable,
    /// A raw or lossless-filtered image (`FlateDecode`, `LZWDecode`, or none):
    /// the bytes are pixel samples, not a self-contained image file.
    Raw,
}

impl EmbeddingKind {
    /// The encoding of the image XObject `stream`: the last filter in its chain
    /// determines the sample encoding.
    fn of(stream: &Stream) -> Self {
        let filters = stream.filters().unwrap_or_default();
        match filters.as_slice() {
            [b"DCTDecode"] => Self::Jpeg,
            [b"JPXDecode"] => Self::Jpeg2000,
            [
                ..,
                b"DCTDecode" | b"JPXDecode" | b"CCITTFaxDecode" | b"JBIG2Decode",
            ] => Self::Unreadable,
            _ => Self::Raw,
        }
    }
}

/// Whether the image XObject `stream` can be surfaced to the image pipeline: a
/// JPEG or JPEG 2000 file as it is, or raw samples the decoder reads.
pub(crate) fn is_surfaceable(doc: &Document, stream: &Stream) -> bool {
    match EmbeddingKind::of(stream) {
        EmbeddingKind::Jpeg | EmbeddingKind::Jpeg2000 => true,
        EmbeddingKind::Raw => samples::is_supported(doc, &stream.dict),
        EmbeddingKind::Unreadable => false,
    }
}

/// One embedded image as the image pipeline reads it.
pub(crate) struct SurfacedImage {
    /// The image XObject.
    pub(crate) id: ImageId,
    /// The image file: a JPEG or JPEG 2000 image as it is stored, raw samples
    /// decoded to a PNG.
    pub(crate) bytes: Bytes,
    /// The file's extension, the registry hint.
    pub(crate) hint: &'static str,
}

/// Every image XObject of `doc`, wherever it is drawn from (a page, a form, a
/// pattern, a font glyph, a soft mask), as the image pipeline reads it.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) for an
/// image that cannot be surfaced: the sanitizer removes those at decode, so one
/// left is an error, never an image shipped unscanned.
pub(crate) fn surfaced(doc: &Document) -> Result<Vec<SurfacedImage>> {
    doc.objects
        .iter()
        .filter_map(|(&id, object)| match object {
            Object::Stream(stream) if stream.dict.has_subtype(b"Image") => Some((id, stream)),
            _ => None,
        })
        .map(|((number, generation), stream)| {
            let id = ImageId::new(number, generation);
            let (bytes, hint) = match EmbeddingKind::of(stream) {
                EmbeddingKind::Jpeg => (Bytes::copy_from_slice(&stream.content), "jpg"),
                EmbeddingKind::Jpeg2000 => (Bytes::copy_from_slice(&stream.content), "jp2"),
                EmbeddingKind::Raw => (Bytes::from(samples::png(doc, stream)?), "png"),
                EmbeddingKind::Unreadable => {
                    return Err(Error::malformed(format!(
                        "image ({number}, {generation}) cannot be surfaced"
                    )));
                }
            };
            Ok(SurfacedImage { id, bytes, hint })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use lopdf::content::{Content, Operation};
    use lopdf::dictionary;

    use super::*;

    /// A 1x1 gray image of raw samples.
    fn gray() -> Stream {
        Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
            },
            vec![0],
        )
    }

    /// A one-page document whose page draws a form, which draws an image with a
    /// soft mask: neither image is in the page's own resources.
    fn drawn_through_a_form() -> (Document, ObjectId, ObjectId) {
        let mut doc = Document::with_version("1.5");
        let mask = doc.add_object(gray());
        let mut image = gray();
        image.dict.set("SMask", mask);
        let image = doc.add_object(image);
        let form = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "Resources" => dictionary! { "XObject" => dictionary! { "Im" => image } },
            },
            Content {
                operations: vec![Operation::new("Do", vec!["Im".into()])],
            }
            .encode()
            .unwrap(),
        ));
        let content = doc.add_object(Stream::new(
            dictionary! {},
            Content {
                operations: vec![Operation::new("Do", vec!["Fm".into()])],
            }
            .encode()
            .unwrap(),
        ));
        let pages = doc.new_object_id();
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages, "Contents" => content,
            "Resources" => dictionary! { "XObject" => dictionary! { "Fm" => form } },
        });
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1,
            }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);
        (doc, image, mask)
    }

    #[test]
    fn an_image_drawn_through_a_form_is_surfaced_with_its_mask() {
        let (doc, image, mask) = drawn_through_a_form();
        let mut ids: Vec<ObjectId> = surfaced(&doc)
            .unwrap()
            .into_iter()
            .map(|surfaced| surfaced.id.object())
            .collect();
        ids.sort_unstable();
        let mut expected = vec![image, mask];
        expected.sort_unstable();
        assert_eq!(ids, expected);
    }

    #[test]
    fn a_removed_mask_is_unlinked_from_its_image() {
        let (mut doc, image, mask) = drawn_through_a_form();
        remove(&mut doc, &HashSet::from([mask]), false).unwrap();
        let image = doc.get_object(image).and_then(Object::as_stream).unwrap();
        assert!(image.dict.get(b"SMask").is_err());
        doc.prune_objects();
        assert!(doc.get_object(mask).is_err());
    }
}
