//! A PDF's embedded images: how one is identified and encoded, how it reaches
//! the image pipeline, and how a redacted or unreadable one goes back.
//!
//! Every image XObject surfaces as the file the pipeline reads, wherever it is
//! drawn from ([`Images`]): a JPEG or JPEG 2000 image as it is stored, raw
//! samples decoded to a PNG. One that cannot surface (fax or
//! JBIG2 samples, a JPEG under a further filter, samples of an unsupported
//! kind, a stencil mask, an inline image) is left behind by the rebuild at
//! decode. An [`ImageRedaction`] writes a redacted image back over its XObject
//! and removes one no codec could read.

mod read;
mod redact;
mod samples;

use std::collections::HashSet;

use bytes::Bytes;
use elide_core::Result;

pub(crate) use self::read::{is_surfaceable, raw_samples};
pub(crate) use self::samples::stencil_extent;
use crate::pdf::reader::ObjectId;
use crate::pdf::{Edit, Pdf, Read, Redact};

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

    /// The image that is the object `id`.
    pub(crate) fn of(id: ObjectId) -> Self {
        Self::new(id.obj_number, u16::try_from(id.gen_number).unwrap_or(0))
    }

    /// The object this image is.
    pub(crate) fn object(self) -> ObjectId {
        ObjectId::new(self.number, i32::from(self.generation))
    }
}

/// One image replacement: overwrite the image XObject [`id`](ImageReplacement::id)
/// with a redacted image.
///
/// The `image` bytes are a self-contained encoded image (PNG or JPEG), the same
/// form a caller extracts, redacts, and re-encodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImageReplacement {
    /// The image to replace.
    pub(crate) id: ImageId,
    /// The redacted image, encoded as PNG or JPEG.
    pub(crate) image: Vec<u8>,
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

/// Every image XObject of the document, wherever it is drawn from (a page, a
/// form, a pattern, a font glyph, a soft mask), as the image pipeline reads
/// it.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) for an
/// image that cannot be surfaced: the rebuild removes those at decode, so one
/// left is an error, never an image shipped unscanned.
pub(crate) struct Images;

impl Read for Images {
    type Output = Vec<SurfacedImage>;

    fn read(self, pdf: &Pdf) -> Result<Self::Output> {
        read::surfaced(pdf)
    }
}

/// Each image `replaced` names written back with its redacted bytes, and each
/// image `removed` names (one no codec could read) cut from every drawing and
/// dropped.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if a
/// replacement names no image of the document, or
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a
/// replacement cannot be decoded or a content stream rewritten.
pub(crate) struct ImageRedaction<'a> {
    pub(crate) replaced: &'a [ImageReplacement],
    pub(crate) removed: &'a [ImageId],
}

impl Redact for ImageRedaction<'_> {
    fn redact(self, edit: &mut Edit) -> Result<()> {
        let removed: HashSet<ObjectId> = self.removed.iter().map(|id| id.object()).collect();
        redact::redact(edit, self.replaced, &removed)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use hayro_syntax::object::Object;

    use super::*;
    use crate::fixtures::{open, read_id, with_pages};
    use crate::pdf::Edit;
    use crate::pdf::content::{Operation, encode};
    use crate::pdf::object::{Id, Stream, Value, dict};
    use crate::pdf::reader::DictExt;
    use crate::pdf::writer::{Changes, Document};
    use crate::sanitize::{Check, Rebuild};

    /// A 1x1 gray image of raw samples.
    fn gray() -> Stream {
        Stream::new(
            dict! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
            },
            vec![0],
        )
    }

    /// A one-page document whose page draws a form, which draws an image with a
    /// soft mask: neither image is in the page's own resources.
    fn drawn_through_a_form() -> (Document, Id, Id) {
        let mut doc = Document::new((1, 5));
        let mask = doc.add(gray());
        let mut image = gray();
        image.dict.set("SMask", mask);
        let image = doc.add(image);
        let form = doc.add(Stream::new(
            dict! {
                "Type" => "XObject", "Subtype" => "Form",
                "Resources" => dict! { "XObject" => dict! { "Im" => image } },
            },
            encode(&[Operation::new("Do", vec!["Im".into()])]).unwrap(),
        ));
        let content = doc.add(Stream::new(
            dict! {},
            encode(&[Operation::new("Do", vec!["Fm".into()])]).unwrap(),
        ));
        with_pages(
            &mut doc,
            [dict! {
                "Contents" => content,
                "Resources" => dict! { "XObject" => dict! { "Fm" => form } },
            }],
        );
        (doc, image, mask)
    }

    /// `pdf` with the images `images` removed, and how many drawings were
    /// cut.
    fn removed(pdf: &Pdf, images: &HashSet<ObjectId>) -> (Pdf, usize) {
        let mut changes = Changes::new();
        let cuts = redact::remove(pdf, images, &mut changes).unwrap();
        let mut edit = Edit::of(pdf);
        edit.apply(changes).unwrap();
        (edit.finish(), cuts)
    }

    #[test]
    fn an_image_drawn_through_a_form_is_surfaced_with_its_mask() {
        let (doc, image, mask) = drawn_through_a_form();
        let pdf = open(&doc);
        let mut ids: Vec<ObjectId> = read::surfaced(&pdf)
            .unwrap()
            .into_iter()
            .map(|surfaced| surfaced.id.object())
            .collect();
        ids.sort_unstable();
        let mut expected = vec![read_id(image), read_id(mask)];
        expected.sort_unstable();
        assert_eq!(ids, expected);
    }

    /// An image a tiling pattern or a Type3 glyph draws is cut from that
    /// stream too, so its drawing never names a resource that is gone. The
    /// font is held in place in the source; the rebuild, which image removal
    /// always follows, makes it an object of its own.
    #[test]
    fn an_image_drawn_by_a_pattern_or_glyph_is_cut() {
        let mut doc = Document::new((1, 5));
        let image = doc.add(gray());
        let draw = || encode(&[Operation::new("Do", vec!["Im".into()])]).unwrap();
        let resources = || dict! { "XObject" => dict! { "Im" => image } };
        let pattern = doc.add(Stream::new(
            dict! {
                "Type" => "Pattern", "PatternType" => 1, "PaintType" => 1, "TilingType" => 1,
                "BBox" => vec![0.into(), 0.into(), 1.into(), 1.into()],
                "XStep" => 1, "YStep" => 1, "Resources" => resources(),
            },
            draw(),
        ));
        let glyph = doc.add(Stream::new(dict! {}, draw()));
        let font = Value::Dict(dict! {
            "Type" => "Font", "Subtype" => "Type3",
            "FontBBox" => vec![0.into(), 0.into(), 1.into(), 1.into()],
            "FontMatrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 0.into(), 0.into()],
            "CharProcs" => dict! { "a" => glyph },
            "Encoding" => dict! { "Differences" => vec![97.into(), Value::name("a")] },
            "FirstChar" => 97, "LastChar" => 97, "Widths" => vec![1.into()],
            "Resources" => resources(),
        });
        let content = doc.add(Stream::new(
            dict! {},
            b"/Pattern cs /P scn 0 0 1 1 re f BT /T3 1 Tf (a) Tj ET".to_vec(),
        ));
        with_pages(
            &mut doc,
            [dict! {
                "Contents" => content,
                "MediaBox" => vec![0.into(), 0.into(), 10.into(), 10.into()],
                "Resources" => dict! {
                    "Pattern" => dict! { "P" => pattern },
                    "Font" => dict! { "T3" => font },
                },
            }],
        );
        let rebuilt = open(&doc).read(Rebuild).unwrap().pdf;
        let image = rebuilt
            .file()
            .objects()
            .into_iter()
            .filter_map(Object::into_stream)
            .find(|stream| stream.dict().is_subtype(b"Image"))
            .map(|stream| stream.obj_id())
            .unwrap();
        let (pdf, cuts) = removed(&rebuilt, &HashSet::from([image]));
        assert_eq!(cuts, 2);
        assert!(pdf.object(image).is_none());
        pdf.read(Check).unwrap();
    }

    #[test]
    fn a_removed_mask_is_unlinked_from_its_image() {
        let (doc, image, mask) = drawn_through_a_form();
        let pdf = open(&doc);
        let (pdf, _) = removed(&pdf, &HashSet::from([read_id(mask)]));
        let drawn = pdf.object(read_id(image)).unwrap().into_stream().unwrap();
        assert!(!drawn.dict().contains_key(b"SMask"));
        assert!(pdf.object(read_id(mask)).is_none());
    }
}
