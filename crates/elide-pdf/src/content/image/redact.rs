//! Redacting a PDF's embedded images: a redacted image written back over its
//! XObject, and an image no codec could read cut from every drawing.
//!
//! A replacement's XObject is rebuilt from the encoded image the pipeline
//! hands back, so its stream and dictionary (dimensions, colour space,
//! filter) stay consistent and the redacted pixels genuinely replace the
//! original. A removed image leaves its area empty, since a page positions
//! everything it draws.

use std::collections::HashSet;

use ::image::{ColorType, DynamicImage, ImageFormat, guess_format, load_from_memory};
use elide_core::{Error, Result};
use hayro_syntax::object::{Object, Stream as Source};

use super::ImageReplacement;
use super::samples::is_stencil;
use crate::pdf::content::{Operation, encode, parse};
use crate::pdf::object::{Stream, Value, dict};
use crate::pdf::reader::{DictExt, ObjectId, Scope};
use crate::pdf::writer::{Changes, compressed, drop_xobjects, rewrite_contents, stream_value};
use crate::pdf::{Edit, Pdf};

/// Replace in `changes` the embedded images of `pdf` named in
/// `replacements` with redacted images.
///
/// Each [`ImageReplacement`] carries an encoded image (PNG/JPEG); its XObject is
/// rebuilt so the stream and dictionary stay consistent. The original
/// image's sub-objects that encode pixel content (soft masks, stencil masks,
/// alternate representations) are no longer named by the rebuilt XObject,
/// so they are not written, unless a retained object still names them.
///
/// **Fail-closed:** a replacement naming an object that is not an image XObject,
/// or an image that cannot be decoded, refuses the whole rewrite rather than
/// emitting a document with a broken or unredacted image.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if a replacement could
/// not be applied.
pub(super) fn replace(
    pdf: &Pdf,
    replacements: &[ImageReplacement],
    changes: &mut Changes,
) -> Result<()> {
    for replacement in replacements {
        let id = replacement.id.object();
        // The target must be an image XObject, not a content or font stream,
        // or a structural object, that happens to share the id.
        let Some(image) = image(pdf, id) else {
            return Err(Error::redaction(format!(
                "object ({}, {}) is not a replaceable image XObject",
                replacement.id.number, replacement.id.generation
            )));
        };
        // A stencil mask stays a stencil, so it still paints in the fill
        // colour.
        let built = if is_stencil(image.dict()) {
            stencil_from(&replacement.image)
        } else {
            image_from(&replacement.image, changes)
        };
        let stream = built.map_err(|e| {
            Error::redaction(format!(
                "could not build image ({}, {}): {e}",
                replacement.id.number, replacement.id.generation
            ))
        })?;
        changes.replace(id, stream);
    }
    Ok(())
}

/// An image XObject of the encoded image `bytes`: a JPEG as it is, any
/// other image as its samples, its alpha (when it has one) a soft mask
/// added to `changes`.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if the bytes are
/// not an image, and as [`compressed`].
fn image_from(bytes: &[u8], changes: &mut Changes) -> Result<Stream> {
    let image =
        load_from_memory(bytes).map_err(|e| Error::redaction(format!("not an image: {e}")))?;
    let (width, height) = (image.width(), image.height());
    let gray = matches!(
        image.color(),
        ColorType::L8 | ColorType::L16 | ColorType::La8 | ColorType::La16
    );
    let space = if gray { "DeviceGray" } else { "DeviceRGB" };
    let mut dict = dict! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => width,
        "Height" => height,
        "ColorSpace" => space,
        "BitsPerComponent" => 8,
    };
    // A JPEG of gray or RGB samples is stored as it is.
    let jpeg = guess_format(bytes).is_ok_and(|format| format == ImageFormat::Jpeg)
        && matches!(image.color(), ColorType::L8 | ColorType::Rgb8);
    if jpeg {
        dict.set("Filter", "DCTDecode");
        return Ok(Stream::new(dict, bytes.to_vec()));
    }
    if image.color().has_alpha() {
        let alpha: Vec<u8> = image.to_rgba8().pixels().map(|pixel| pixel.0[3]).collect();
        let mask = compressed(
            dict! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => width,
                "Height" => height,
                "ColorSpace" => "DeviceGray",
                "BitsPerComponent" => 8,
            },
            &alpha,
        )?;
        dict.set("SMask", changes.add(mask).value());
    }
    let samples = match image {
        image if gray => DynamicImage::ImageLuma8(image.to_luma8()).into_bytes(),
        image => DynamicImage::ImageRgb8(image.to_rgb8()).into_bytes(),
    };
    compressed(dict, &samples)
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
        dict! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => width,
            "Height" => height,
            "ImageMask" => true,
            "BitsPerComponent" => 1,
        },
        &samples,
    )
}

/// The image XObject `id` of `pdf`, if it is one.
fn image(pdf: &Pdf, id: ObjectId) -> Option<Source<'_>> {
    pdf.object(id)
        .and_then(Object::into_stream)
        .filter(|stream| stream.dict().is_subtype(b"Image"))
}

/// Remove in `changes` the image XObjects `images` from every page and form
/// XObject of `pdf`, returning how many drawings were cut. Each is dropped
/// from every resource dictionary, and from every image it masks, so the
/// document written no longer holds it.
///
/// A page positions everything it draws, so cutting an image leaves its area
/// empty without moving anything else.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a
/// content stream cannot be decoded or encoded.
pub(super) fn remove(
    pdf: &Pdf,
    images: &HashSet<ObjectId>,
    changes: &mut Changes,
) -> Result<usize> {
    let cuts = rewrite_contents(pdf, changes, |bytes, scope| {
        let content = parse(bytes);
        let before = content.len();
        let kept: Vec<Operation> = content
            .into_iter()
            .filter(|operation| operation.operator != "Do" || !draws(operation, scope, images))
            .collect();
        let cut = before - kept.len();
        if cut == 0 {
            return Ok(None);
        }
        Ok(Some((encode(&kept)?, cut)))
    })?;
    drop_xobjects(pdf, images, changes);
    drop_masks(pdf, images, changes);
    Ok(cuts.into_iter().sum())
}

/// Drop in `changes` every `/SMask` and `/Mask` entry of `pdf` naming one of
/// `images`.
fn drop_masks(pdf: &Pdf, images: &HashSet<ObjectId>, changes: &mut Changes) {
    for object in pdf.file().objects() {
        let Object::Stream(stream) = object else {
            continue;
        };
        let masked: Vec<&[u8]> = [b"SMask".as_slice(), b"Mask"]
            .into_iter()
            .filter(|key| {
                stream
                    .dict()
                    .reference(key)
                    .is_some_and(|id| images.contains(&id))
            })
            .collect();
        if masked.is_empty() {
            continue;
        }
        let written = changes.entry(stream.obj_id(), || Value::from(stream_value(&stream)));
        if let Some(dict) = written.as_dict_mut() {
            for key in masked {
                dict.remove(key);
            }
        }
    }
}

/// Whether the `Do` `operation` draws one of `images`, by its name in
/// `scope`.
fn draws(operation: &Operation, scope: &Scope<'_>, images: &HashSet<ObjectId>) -> bool {
    let [Value::Name(name)] = operation.operands.as_slice() else {
        return false;
    };
    scope
        .lookup(b"XObject", name)
        .and_then(|xobject| xobject.id)
        .is_some_and(|id| images.contains(&id))
}

/// Replace the images `replaced` names and remove those `removed` names, in
/// one change of `edit`.
///
/// # Errors
///
/// As [`replace`] and [`remove`], and as [`Edit::apply`].
pub(super) fn redact(
    edit: &mut Edit,
    replaced: &[ImageReplacement],
    removed: &HashSet<ObjectId>,
) -> Result<()> {
    let mut changes = Changes::new();
    replace(edit.pdf(), replaced, &mut changes)?;
    if !removed.is_empty() {
        remove(edit.pdf(), removed, &mut changes)?;
    }
    edit.apply(changes)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use ::image::{DynamicImage, GrayImage, ImageFormat, Luma};

    use super::*;
    use crate::content::image::ImageId;
    use crate::content::image::samples::decode;
    use crate::fixtures::{held, read_id};
    use crate::pdf::writer::Document;

    #[test]
    fn only_an_image_xobject_is_replaced() {
        let mut doc = Document::new((1, 5));
        let page = doc.add(dict! { "Type" => "Page" });
        let font = doc.add(dict! { "Type" => "Font", "Subtype" => "Type1" });
        let picture = doc.add(Stream::new(
            dict! { "Type" => "XObject", "Subtype" => "Image" },
            b"pixels".to_vec(),
        ));
        let pdf = held(doc, &[page, font, picture]);
        for id in [page, font] {
            assert!(image(&pdf, read_id(id)).is_none());
        }
        assert!(image(&pdf, read_id(picture)).is_some());
    }

    /// A redacted stencil mask goes back as a stencil: what was painted
    /// paints, and the redaction box (black) paints too.
    #[test]
    fn a_stencil_stays_a_stencil() {
        let mut doc = Document::new((1, 5));
        let id = doc.add(Stream::new(
            dict! {
                "Type" => "XObject", "Subtype" => "Image",
                "Width" => 2, "Height" => 1, "ImageMask" => true,
            },
            vec![0b0100_0000],
        ));
        let pdf = held(doc, &[id]);
        let redacted = GrayImage::from_fn(2, 1, |_, _| Luma([0]));
        let mut png = Vec::new();
        DynamicImage::ImageLuma8(redacted)
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        let mut edit = Edit::of(&pdf);
        redact(
            &mut edit,
            &[ImageReplacement {
                id: ImageId::new(id.0, 0),
                image: png,
            }],
            &HashSet::new(),
        )
        .unwrap();
        let pdf = edit.finish();
        let stream = pdf.object(read_id(id)).unwrap().into_stream().unwrap();
        assert!(is_stencil(stream.dict()));
        assert_eq!(decode(&stream).unwrap().data, [0, 0]);
    }
}
