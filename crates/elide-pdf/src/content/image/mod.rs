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
use lopdf::ObjectId;

pub(crate) use self::read::{is_surfaceable, raw_samples};
pub(crate) use self::samples::stencil_extent;
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

    /// This id as a lopdf `ObjectId` pair.
    pub(crate) fn object(self) -> ObjectId {
        (self.number, self.generation)
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
        read::surfaced(pdf.doc())
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
        redact::replace(edit, self.replaced)?;
        if !self.removed.is_empty() {
            let removed: HashSet<ObjectId> = self.removed.iter().map(|id| id.object()).collect();
            redact::remove(edit, &removed)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use lopdf::{Dictionary, Document, Object, Stream, dictionary};

    use super::*;
    use crate::fixtures::{DocBuilder, Page};
    use crate::pdf::reader::DictionaryExt;
    use crate::sanitize::{Check, Rebuild};

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
        let mut pdf = DocBuilder::with_version("1.5");
        let mask = pdf.add(gray());
        let mut image = gray();
        image.dict.set("SMask", mask);
        let image = pdf.add(image);
        let form = pdf.form(
            dictionary! { "Resources" => dictionary! { "XObject" => dictionary! { "Im" => image } } },
            b"/Im Do".to_vec(),
        );
        pdf.page(Page::new().xobject("Fm", form).content(b"/Fm Do".to_vec()));
        (pdf.finish().0, image, mask)
    }

    #[test]
    fn an_image_drawn_through_a_form_is_surfaced_with_its_mask() {
        let (doc, image, mask) = drawn_through_a_form();
        let mut ids: Vec<ObjectId> = read::surfaced(&doc)
            .unwrap()
            .into_iter()
            .map(|surfaced| surfaced.id.object())
            .collect();
        ids.sort_unstable();
        let mut expected = vec![image, mask];
        expected.sort_unstable();
        assert_eq!(ids, expected);
    }

    /// An image a tiling pattern or a Type3 glyph draws is cut from that
    /// stream too, so its drawing never names a resource that is gone. The
    /// font is held in place in the source; the rebuild, which image removal
    /// always follows, makes it an object of its own.
    #[test]
    fn an_image_drawn_by_a_pattern_or_glyph_is_cut() {
        let mut pdf = DocBuilder::with_version("1.5");
        let image = pdf.add(gray());
        let draw = || b"/Im Do".to_vec();
        let resources = || dictionary! { "XObject" => dictionary! { "Im" => image } };
        let pattern = pdf.add(Stream::new(
            dictionary! {
                "Type" => "Pattern", "PatternType" => 1, "PaintType" => 1, "TilingType" => 1,
                "BBox" => vec![0.into(), 0.into(), 1.into(), 1.into()],
                "XStep" => 1, "YStep" => 1, "Resources" => resources(),
            },
            draw(),
        ));
        let glyph = pdf.stream(Dictionary::new(), draw());
        let font = Object::Dictionary(dictionary! {
            "Type" => "Font", "Subtype" => "Type3",
            "FontBBox" => vec![0.into(), 0.into(), 1.into(), 1.into()],
            "FontMatrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 0.into(), 0.into()],
            "CharProcs" => dictionary! { "a" => glyph },
            "Encoding" => dictionary! { "Differences" => vec![97.into(), Object::from("a")] },
            "FirstChar" => 97, "LastChar" => 97, "Widths" => vec![1.into()],
            "Resources" => resources(),
        });
        pdf.page(
            Page::new()
                .resource("Pattern", "P", pattern)
                .font("T3", font)
                .content(b"/Pattern cs /P scn 0 0 1 1 re f BT /T3 1 Tf (a) Tj ET".to_vec())
                .size(10, 10),
        );
        let bytes = pdf.bytes();
        let rebuilt = Pdf::open(&bytes).unwrap().read(Rebuild).unwrap().pdf;
        let image = rebuilt
            .doc()
            .objects
            .iter()
            .find(|(_, object)| {
                object
                    .as_stream()
                    .is_ok_and(|s| s.dict.is_subtype(rebuilt.doc(), b"Image"))
            })
            .map(|(&id, _)| id)
            .unwrap();
        let mut edit = rebuilt.edit();
        assert_eq!(
            redact::remove(&mut edit, &HashSet::from([image])).unwrap(),
            2
        );
        let pdf = edit.finish().unwrap();
        assert!(pdf.doc().get_object(image).is_err());
        pdf.read(Check).unwrap();
    }

    #[test]
    fn a_removed_mask_is_unlinked_from_its_image() {
        let (doc, image, mask) = drawn_through_a_form();
        let mut edit = Edit::of(doc);
        redact::remove(&mut edit, &HashSet::from([mask])).unwrap();
        let drawn = edit
            .doc()
            .get_object(image)
            .and_then(Object::as_stream)
            .unwrap();
        assert!(drawn.dict.get(b"SMask").is_err());
        let pdf = edit.finish().unwrap();
        assert!(pdf.doc().get_object(mask).is_err());
    }
}
