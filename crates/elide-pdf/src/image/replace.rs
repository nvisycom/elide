//! Replace an embedded image XObject with a redacted image, rebuilding a
//! valid XObject from encoded bytes.
//!
//! A caller extracts an [`Embedding`](crate::extract::Embedding)'s bytes,
//! redacts the image (blanking the sensitive region), re-encodes it, and hands
//! the result back as an [`ImageReplacement`]. The XObject is then rebuilt from
//! those encoded bytes so its stream and dictionary (dimensions, colour space,
//! filter) stay consistent, the redacted pixels genuinely replace the original.

use elide_core::{Error, ErrorKind, Result};
use lopdf::{Document, Object, ObjectId};

use super::ImageId;
use crate::lopdf_ext::DictionaryExt;

/// One image replacement: overwrite the image XObject [`id`](ImageReplacement::id)
/// with a redacted image.
///
/// The `image` bytes are a self-contained encoded image (PNG or JPEG), the same
/// form a caller extracts, redacts, and re-encodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageReplacement {
    /// The image to replace.
    pub id: ImageId,
    /// The redacted image, encoded as PNG or JPEG.
    pub image: Vec<u8>,
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
/// [`ErrorKind::Redaction`] if a replacement could
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
            return Err(Error::new(
                ErrorKind::Redaction,
                format!(
                    "object ({}, {}) is not a replaceable image XObject",
                    replacement.id.number, replacement.id.generation
                ),
            ));
        }

        // Rebuild a valid image XObject from the encoded bytes (lopdf sets
        // the dictionary, dimensions, colour space, filter, to match).
        let stream = lopdf::xobject::image_from(replacement.image.clone()).map_err(|e| {
            Error::new(
                ErrorKind::Redaction,
                format!(
                    "could not build image ({}, {}): {e}",
                    replacement.id.number, replacement.id.generation
                ),
            )
        })?;
        doc.objects.insert(id, Object::Stream(stream));
    }
    Ok(())
}

/// Whether `id` is an image XObject of `doc`.
fn is_image(doc: &Document, id: ObjectId) -> bool {
    matches!(doc.get_object(id), Ok(Object::Stream(stream)) if stream.dict.has_subtype(b"Image"))
}

#[cfg(test)]
mod tests {
    use lopdf::{Stream, dictionary};

    use super::*;

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
}
