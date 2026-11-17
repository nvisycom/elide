//! Replace an embedded image XObject with a redacted image, rebuilding a
//! valid XObject from encoded bytes.
//!
//! A caller extracts an [`Embedding`](crate::document::extract::Embedding)'s bytes,
//! redacts the image (blanking the sensitive region), re-encodes it, and hands
//! the result back as an [`ImageReplacement`]. The XObject is then rebuilt from
//! those encoded bytes so its stream and dictionary (dimensions, colour space,
//! filter) stay consistent, the redacted pixels genuinely replace the original.

use ::image::load_from_memory;
use elide_core::{Error, Result};
use lopdf::xobject::image_from;
use lopdf::{Document, Object, ObjectId, Stream, dictionary};

use super::ImageId;
use super::samples::is_stencil;
use crate::document::object::DictionaryExt;

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

/// Replace the embedded images named in `replacements` in `doc` with redacted
/// images.
///
/// Each [`ImageReplacement`] carries an encoded image (PNG/JPEG); its XObject is
/// rebuilt so the stream and dictionary stay consistent.
///
/// **Fail-closed:** a replacement naming an object that is not an image XObject,
/// or an image that cannot be decoded, refuses the whole rewrite rather than
/// emitting a document with a broken or unredacted image.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if a replacement could
/// not be applied.
pub(crate) fn replace(doc: &mut Document, replacements: &[ImageReplacement]) -> Result<()> {
    // The original images' sub-objects that encode pixel content (soft masks,
    // stencil masks, alternate representations) become unreachable when the
    // rebuilt XObject drops their dict keys, and the save prunes them, so no
    // mask carrying the sensitive shape survives in the bytes. A mask shared
    // with a retained object stays reachable and is kept.
    for replacement in replacements {
        let id = replacement.id.object();

        // The target must be an image XObject, not a content or font stream,
        // or a structural object, that happens to share the id.
        if !is_image(doc, id) {
            return Err(Error::redaction(format!(
                "object ({}, {}) is not a replaceable image XObject",
                replacement.id.number, replacement.id.generation
            )));
        }

        // Rebuild a valid image XObject from the encoded bytes (lopdf sets
        // the dictionary, dimensions, colour space, filter, to match); a
        // stencil mask stays a stencil, so it still paints in the fill colour.
        let stencil = matches!(doc.get_object(id), Ok(Object::Stream(s)) if is_stencil(&s.dict));
        let stream = if stencil {
            stencil_from(&replacement.image)
        } else {
            image_from(replacement.image.clone()).map_err(|e| Error::redaction(e.to_string()))
        }
        .map_err(|e| {
            Error::redaction(format!(
                "could not build image ({}, {}): {e}",
                replacement.id.number, replacement.id.generation
            ))
        })?;
        doc.objects.insert(id, Object::Stream(stream));
    }
    Ok(())
}

/// A stencil mask of the encoded image `bytes`: a dark pixel paints, a light
/// one leaves the page as it is.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if the bytes are
/// not an image, or the mask cannot be compressed.
fn stencil_from(bytes: &[u8]) -> Result<Stream> {
    let image = load_from_memory(bytes)
        .map_err(|e| Error::redaction(format!("not an image: {e}")))?
        .into_luma8();
    let (width, height) = image.dimensions();
    let row_bytes = (width as usize).div_ceil(8);
    let mut samples = vec![0u8; row_bytes * height as usize];
    for (x, y, pixel) in image.enumerate_pixels() {
        // Under the default decode array a 0 sample paints, so a light pixel
        // is the set bit.
        if pixel.0[0] >= 128 {
            samples[y as usize * row_bytes + x as usize / 8] |= 0x80 >> (x % 8);
        }
    }
    let mut stream = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => i64::from(width),
            "Height" => i64::from(height),
            "ImageMask" => true,
            "BitsPerComponent" => 1,
        },
        samples,
    );
    stream
        .compress()
        .map_err(|e| Error::redaction(format!("could not compress mask: {e}")))?;
    Ok(stream)
}

/// Whether `id` is an image XObject of `doc`.
fn is_image(doc: &Document, id: ObjectId) -> bool {
    matches!(doc.get_object(id), Ok(Object::Stream(stream)) if stream.dict.has_subtype(b"Image"))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use ::image::{DynamicImage, GrayImage, ImageFormat, Luma};

    use super::*;
    use crate::document::image::samples::decode;

    #[test]
    fn only_an_image_xobject_is_replaced() {
        let mut doc = Document::with_version("1.5");
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog" });
        let page = doc.add_object(dictionary! { "Type" => "Page" });
        let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1" });
        let image = doc.add_object(Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Image" },
            b"pixels".to_vec(),
        ));
        for id in [catalog, page, font] {
            assert!(!is_image(&doc, id));
        }
        assert!(is_image(&doc, image));
    }

    /// A redacted stencil mask goes back as a stencil: what was painted
    /// paints, and the redaction box (black) paints too.
    #[test]
    fn a_stencil_stays_a_stencil() {
        let mut doc = Document::with_version("1.5");
        let id = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image",
                "Width" => 2, "Height" => 1, "ImageMask" => true,
            },
            vec![0b0100_0000],
        ));
        let redacted = GrayImage::from_fn(2, 1, |_, _| Luma([0]));
        let mut png = Vec::new();
        DynamicImage::ImageLuma8(redacted)
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        replace(
            &mut doc,
            &[ImageReplacement {
                id: ImageId::new(id.0, id.1),
                image: png,
            }],
        )
        .unwrap();
        let stream = doc.get_object(id).and_then(Object::as_stream).unwrap();
        assert!(is_stencil(&stream.dict));
        assert_eq!(decode(&doc, stream).unwrap().data, [0, 0]);
    }
}
