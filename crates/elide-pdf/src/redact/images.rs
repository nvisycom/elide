//! Image redaction: replace an embedded image XObject with a redacted image,
//! rebuilding a valid XObject from encoded bytes.
//!
//! A caller extracts an [`Embedding`](crate::extract::Embedding)'s bytes,
//! redacts the image (blanking the sensitive region), re-encodes it, and hands
//! the result back as an [`ImageReplacement`]. The XObject is then rebuilt from
//! those encoded bytes so its stream and dictionary (dimensions, colour space,
//! filter) stay consistent, the redacted pixels genuinely replace the original.
//!
//! Behind the `image` feature, which pulls the `image` crate (via lopdf's
//! `embed_image`) to build the XObject; the default build carries no image
//! dependency.

use elide_core::{Error, ErrorKind, Result};
use lopdf::Object;

use crate::document::{ObjectRole, Store, save};
use crate::extract::ImageId;

/// One image replacement: overwrite the image XObject [`id`](ImageReplacement::id)
/// with a redacted image.
///
/// The `image` bytes are a self-contained encoded image (PNG or JPEG), the same
/// form a caller extracts, redacts, and re-encodes.
#[cfg_attr(docsrs, doc(cfg(feature = "image")))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageReplacement {
    /// The image to replace.
    pub id: ImageId,
    /// The redacted image, encoded as PNG or JPEG.
    pub image: Vec<u8>,
}

/// Replace the embedded images named in `replacements` with redacted images,
/// returning the new document bytes.
///
/// Each [`ImageReplacement`] carries an encoded image (PNG/JPEG); its XObject is
/// rebuilt so the stream and dictionary stay consistent. The rest of the
/// document is re-saved unchanged.
///
/// **Fail-closed:** a replacement naming an object that is not an image XObject,
/// or an image that cannot be decoded, refuses the whole rewrite rather than
/// emitting a document with a broken or unredacted image.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](crate::ErrorKind::Redaction) if a replacement could
/// not be applied.
pub(crate) fn redact_images(store: &Store, replacements: &[ImageReplacement]) -> Result<Vec<u8>> {
    let mut doc = store.clone_doc();

    // The original images' sub-objects that encode pixel content (soft masks,
    // stencil masks, alternate representations) become unreachable when the
    // rebuilt XObject drops their dict keys, and the save prunes them, so no
    // mask carrying the sensitive shape survives in the bytes. A mask shared
    // with a retained object stays reachable and is kept.
    for replacement in replacements {
        let id = replacement.id.object();

        // The target must be an image XObject the role seam classifies as
        // whole-object-replaceable, not a content or font stream, or a
        // structural object, that happens to share the id.
        if !ObjectRole::of(&doc, id).is_whole_object_replaceable() {
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

    save(&mut doc)
}
