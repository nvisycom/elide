//! Image redaction: replace an embedded image XObject with a redacted image,
//! rebuilding a valid XObject from encoded bytes.
//!
//! A caller extracts an [`Embedding`](crate::extract::Embedding)'s bytes,
//! redacts the image (blanking the sensitive region), re-encodes it, and hands
//! the result back as an [`ImageReplacement`]. The XObject is then rebuilt from
//! those encoded bytes so its stream and dictionary (dimensions, colour space,
//! filter) stay consistent — the redacted pixels genuinely replace the original.
//!
//! Behind the `image` feature, which pulls the `image` crate (via lopdf's
//! `embed_image`) to build the XObject; the default build carries no image
//! dependency.

use std::collections::BTreeSet;

use lopdf::{Object, ObjectId};

use super::sanitize::referenced_from_survivors;
use crate::error::{Error, Result};
use crate::extract::ImageId;

/// One image replacement: overwrite the image XObject [`id`](ImageReplacement::id)
/// with a redacted image.
///
/// The `image` bytes are a self-contained encoded image (PNG or JPEG) — the same
/// form a caller extracts, redacts, and re-encodes.
#[cfg_attr(docsrs, doc(cfg(feature = "image")))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageReplacement {
    /// The image to replace.
    pub id: ImageId,
    /// The redacted image, encoded as PNG or JPEG.
    pub image: Vec<u8>,
}

impl crate::Pdf {
    /// Replace the embedded images named in `replacements` with redacted images,
    /// returning the new document bytes.
    ///
    /// Each [`ImageReplacement`] carries an encoded image (PNG/JPEG); its XObject
    /// is rebuilt so the stream and dictionary stay consistent. The rest of the
    /// document is re-saved unchanged.
    ///
    /// **Fail-closed:** a replacement naming an object that is not an image
    /// XObject, or an image that cannot be decoded, refuses the whole rewrite
    /// rather than emitting a document with a broken or unredacted image.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::UnsafeRewrite`](crate::ErrorKind::UnsafeRewrite) if a
    /// replacement could not be applied.
    #[cfg_attr(docsrs, doc(cfg(feature = "image")))]
    pub fn redact_images(&self, replacements: &[ImageReplacement]) -> Result<Vec<u8>> {
        let mut doc = self.doc.clone();

        // The original images' sub-objects that encode pixel content — soft
        // masks, stencil masks, alternate representations — become orphaned when
        // the rebuilt XObject drops these dict keys. They must be deleted (else
        // `save_to` still serialises them, leaving a mask that carries the
        // sensitive shape in the bytes), but only if no *surviving* object still
        // references them: a mask shared with a retained object would corrupt
        // the PDF if removed. So gather candidates now, insert all replacements,
        // then prune with a survivor-reference guard.
        let mut orphans: BTreeSet<ObjectId> = BTreeSet::new();

        for replacement in replacements {
            let id = replacement.id.object();

            // The target must already be an image XObject — not a content or
            // font stream that happens to share the id.
            let old = doc.get_object(id).ok().and_then(|o| o.as_stream().ok());
            let is_image = old.and_then(|s| s.dict.get(b"Subtype").and_then(Object::as_name).ok())
                == Some(b"Image".as_slice());
            if !is_image {
                return Err(Error::unsafe_rewrite(format!(
                    "object ({}, {}) is not an image XObject",
                    replacement.id.number, replacement.id.generation
                )));
            }

            if let Some(stream) = old {
                orphans.extend(
                    [b"SMask".as_slice(), b"Mask", b"Alternates"]
                        .iter()
                        .filter_map(|key| stream.dict.get(key).and_then(Object::as_reference).ok()),
                );
            }

            // Rebuild a valid image XObject from the encoded bytes (lopdf sets
            // the dictionary — dimensions, colour space, filter — to match).
            let stream = lopdf::xobject::image_from(replacement.image.clone()).map_err(|e| {
                Error::unsafe_rewrite(format!(
                    "could not build image ({}, {}): {e}",
                    replacement.id.number, replacement.id.generation
                ))
            })?;
            doc.objects.insert(id, Object::Stream(stream));
        }

        // With every replacement inserted, an orphan still referenced from a
        // surviving object is shared and must be kept; delete only the rest.
        let referenced_by_survivors = referenced_from_survivors(&doc, &orphans);
        for orphan in &orphans {
            if !referenced_by_survivors.contains(orphan) {
                doc.objects.remove(orphan);
            }
        }

        let mut out = Vec::new();
        doc.save_to(&mut out)
            .map_err(|e| Error::invalid_document(format!("could not save PDF: {e}")))?;
        Ok(out)
    }
}
