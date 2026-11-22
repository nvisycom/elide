//! Redacting a PDF's embedded images: a redacted image written back over its
//! XObject, and an image no codec could read cut from every drawing.
//!
//! A replacement's XObject is rebuilt from the encoded image the pipeline
//! hands back, so its stream and dictionary (dimensions, colour space,
//! filter) stay consistent and the redacted pixels genuinely replace the
//! original. A removed image leaves its area empty, since a page positions
//! everything it draws.

use std::collections::HashSet;

use ::image::load_from_memory;
use elide_core::{Error, Result};
use lopdf::content::{Content, Operation};
use lopdf::xobject::image_from;
use lopdf::{Document, Object, ObjectId, Stream, dictionary};

use super::ImageReplacement;
use super::samples::is_stencil;
use crate::pdf::Edit;
use crate::pdf::reader::{DictionaryExt, Scope, parse_content};
use crate::pdf::writer::{compressed, encoded};

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
pub(super) fn replace(edit: &mut Edit, replacements: &[ImageReplacement]) -> Result<()> {
    let doc = edit.doc_mut();
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
        let stencil =
            matches!(doc.get_object(id), Ok(Object::Stream(s)) if is_stencil(doc, &s.dict));
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
/// not an image, and as [`compressed`].
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
    compressed(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => i64::from(width),
            "Height" => i64::from(height),
            "ImageMask" => true,
            "BitsPerComponent" => 1,
        },
        samples,
    )
}

/// Whether `id` is an image XObject of `doc`.
fn is_image(doc: &Document, id: ObjectId) -> bool {
    matches!(doc.get_object(id), Ok(Object::Stream(stream)) if stream.dict.is_subtype(doc, b"Image"))
}

/// Remove the image XObjects `images` from every page and form XObject
/// `edit` draws, returning how many drawings were cut. Each is dropped from
/// every resource dictionary, and from every image it masks, so saving the
/// edit prunes it from the file.
///
/// A page positions everything it draws, so cutting an image leaves its area
/// empty without moving anything else.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a
/// content stream cannot be decoded or encoded.
pub(super) fn remove(edit: &mut Edit, images: &HashSet<ObjectId>) -> Result<usize> {
    let cuts = edit.rewrite_contents(|doc, bytes, scope| {
        let content = parse_content(bytes)?;
        let before = content.operations.len();
        let kept: Vec<Operation> = content
            .operations
            .into_iter()
            .filter(|operation| operation.operator != "Do" || !draws(doc, operation, scope, images))
            .collect();
        let cut = before - kept.len();
        if cut == 0 {
            return Ok(None);
        }
        Ok(Some((encoded(&Content { operations: kept })?, cut)))
    })?;
    edit.drop_xobjects(images);
    drop_masks(edit, images);
    Ok(cuts.into_iter().sum())
}

/// Drop every `/SMask` and `/Mask` entry naming one of `images`.
fn drop_masks(edit: &mut Edit, images: &HashSet<ObjectId>) {
    let doc = edit.doc_mut();
    for object in doc.objects.values_mut() {
        let Object::Stream(stream) = object else {
            continue;
        };
        for key in [b"SMask".as_slice(), b"Mask"] {
            if stream
                .dict
                .get(key)
                .and_then(Object::as_reference)
                .is_ok_and(|id| images.contains(&id))
            {
                stream.dict.remove(key);
            }
        }
    }
}

/// Whether the `Do` `operation` draws one of `images`, by its name in
/// `scope`.
fn draws(
    doc: &Document,
    operation: &Operation,
    scope: &Scope<'_>,
    images: &HashSet<ObjectId>,
) -> bool {
    let [Object::Name(name)] = operation.operands.as_slice() else {
        return false;
    };
    scope
        .lookup(doc, b"XObject", name)
        .and_then(|xobject| xobject.as_reference().ok())
        .is_some_and(|id| images.contains(&id))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use ::image::{DynamicImage, GrayImage, ImageFormat, Luma};

    use super::*;
    use crate::content::image::ImageId;
    use crate::content::image::samples::decode;

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
        let mut edit = Edit::of(doc);
        replace(
            &mut edit,
            &[ImageReplacement {
                id: ImageId::new(id.0, id.1),
                image: png,
            }],
        )
        .unwrap();
        let doc = edit.doc();
        let stream = doc.get_object(id).and_then(Object::as_stream).unwrap();
        assert!(is_stencil(doc, &stream.dict));
        assert_eq!(decode(doc, stream).unwrap().data, [0, 0]);
    }
}
