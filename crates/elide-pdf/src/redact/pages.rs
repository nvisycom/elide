//! Page reflatten: replace a whole page's content with a single redacted image.
//!
//! A scanned or image-only page carries no born-digital text to delete, so its
//! redaction is a redacted raster of the page. A caller renders the page,
//! redacts the pixels, and hands the encoded image back as a
//! [`PageReplacement`]; the page's content and resources are then rewritten to
//! draw that one image over the whole page, so nothing of the original page
//! content survives underneath, while every other page of the document is left
//! untouched (a born-digital page keeps its selectable, glyph-redacted text).
//!
//! Behind the `image` feature (like [`redact_images`](crate::Pdf::redact_images)),
//! which pulls the `image` crate to build the XObject.

use std::collections::BTreeSet;

use elide_core::{Error, ErrorKind, Result};
use lopdf::{Object, ObjectId, dictionary};

use super::sanitize::referenced_from_survivors;

/// One page reflatten: replace page [`number`](PageReplacement::number)'s whole
/// content with a redacted image of the page.
///
/// The `image` bytes are a self-contained encoded image (PNG or JPEG) of the
/// redacted page.
#[cfg_attr(docsrs, doc(cfg(feature = "image")))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageReplacement {
    /// 1-based page number to reflatten.
    pub number: u32,
    /// The redacted page image, encoded as PNG or JPEG.
    pub image: Vec<u8>,
}

impl crate::Pdf {
    /// Reflatten the pages named in `replacements`: replace each page's whole
    /// content with a single redacted image, returning the new document bytes.
    ///
    /// Only the named pages are rewritten; every other page is re-saved
    /// unchanged, so a mixed document keeps its born-digital pages' selectable
    /// text and flattens only the scanned ones.
    ///
    /// **Fail-closed:** a replacement naming a page that does not exist, or an
    /// image that cannot be decoded into a valid XObject, refuses the whole
    /// rewrite rather than emitting a document with an unredacted page.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Redaction`](crate::ErrorKind::Redaction) if a
    /// replacement could not be applied.
    #[cfg_attr(docsrs, doc(cfg(feature = "image")))]
    pub fn redact_pages(&self, replacements: &[PageReplacement]) -> Result<Vec<u8>> {
        let mut doc = self.doc.clone();
        let pages = doc.get_pages();

        // The replaced pages' old content and resource objects become orphans.
        // An orphaned object is still serialised by `save_to`, so the original
        // page raster would survive in the bytes; gather the objects each
        // replaced page owns and prune them after, guarding shared objects.
        let mut orphans: BTreeSet<ObjectId> = BTreeSet::new();

        for replacement in replacements {
            let Some(&page_id) = pages.get(&replacement.number) else {
                return Err(Error::new(
                    ErrorKind::Redaction,
                    format!("page {} does not exist", replacement.number),
                ));
            };

            // The page's current content and resource subtrees, doomed once the
            // page is repointed at the redacted image.
            collect_page_owned(&doc, page_id, &mut orphans);

            // Build a valid image XObject from the encoded bytes; lopdf sets the
            // dimensions, colour space, and filter to match.
            let image = lopdf::xobject::image_from(replacement.image.clone()).map_err(|e| {
                Error::new(
                    ErrorKind::Redaction,
                    format!("could not build image for page {}: {e}", replacement.number),
                )
            })?;
            let (width, height) = image_dimensions(&image, replacement.number)?;
            let image_id = doc.add_object(image);

            // Draw the image to fill the page: `q W 0 0 H cm /Im0 Do Q`, with the
            // page sized one unit per pixel so the image maps 1:1.
            let content = format!("q {width} 0 0 {height} 0 0 cm /Im0 Do Q");
            let content_id =
                doc.add_object(lopdf::Stream::new(dictionary! {}, content.into_bytes()));
            let resources_id = doc.add_object(dictionary! {
                "XObject" => dictionary! { "Im0" => Object::Reference(image_id) },
            });

            // Repoint the page at the new content and resources, and size its
            // box to the image. The old content/resources become orphans and are
            // dropped by the object-graph write below (they are page-owned, not
            // shared with a surviving page).
            let page = doc.get_dictionary_mut(page_id).map_err(|e| {
                Error::new(
                    ErrorKind::Redaction,
                    format!("page {} is unreadable: {e}", replacement.number),
                )
            })?;
            page.set("Contents", Object::Reference(content_id));
            page.set("Resources", Object::Reference(resources_id));
            // The redacted image is the page as displayed (rendered post-crop
            // and post-rotation), so the flattened page carries no crop or
            // rotation. `MediaBox`, `CropBox`, `Rotate`, and `UserUnit` are all
            // set explicitly, page-local, so an inherited value from an ancestor
            // `Pages` node cannot clip, rotate, or rescale the image, removing
            // only the page's own keys would leave an inherited one effective.
            let box_rect = vec![
                0.into(),
                0.into(),
                i64::from(width).into(),
                i64::from(height).into(),
            ];
            page.set("MediaBox", box_rect.clone());
            page.set("CropBox", box_rect);
            page.set("Rotate", 0);
            page.set("UserUnit", 1);
        }

        // The new content/resources/image were just added, so they are the
        // survivors that keep any shared object alive; prune only orphans not
        // still referenced from outside the doomed set.
        let referenced_by_survivors = referenced_from_survivors(&doc, &orphans);
        for orphan in &orphans {
            if !referenced_by_survivors.contains(orphan) {
                doc.objects.remove(orphan);
            }
        }

        let mut out = Vec::new();
        doc.save_to(&mut out).map_err(|e| {
            Error::new(
                ErrorKind::MalformedInput,
                format!("could not save PDF: {e}"),
            )
        })?;
        Ok(out)
    }
}

/// Collect the object ids a page owns through its `/Contents` and `/Resources`
/// (and everything those reference), the subtrees replaced when the page is
/// reflattened.
fn collect_page_owned(doc: &lopdf::Document, page_id: ObjectId, out: &mut BTreeSet<ObjectId>) {
    let Ok(page) = doc.get_dictionary(page_id) else {
        return;
    };
    for key in [b"Contents".as_slice(), b"Resources"] {
        if let Ok(entry) = page.get(key) {
            for id in reachable(doc, entry) {
                out.insert(id);
            }
        }
    }
}

/// Every object id reachable from `entry` (a reference, or an inline
/// dictionary/array/stream that references others), transitively.
fn reachable(doc: &lopdf::Document, entry: &Object) -> BTreeSet<ObjectId> {
    let mut seen = BTreeSet::new();
    let mut stack: Vec<ObjectId> = direct_refs(entry);
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let Ok(obj) = doc.get_object(id) {
            stack.extend(direct_refs(obj));
        }
    }
    seen
}

/// The object ids referenced by `object`, recursing into inline dictionaries
/// and arrays (a page's `/Resources` nests its `/XObject` references one inline
/// dictionary deep, so a shallow scan would miss them).
fn direct_refs(object: &Object) -> Vec<ObjectId> {
    let mut out = Vec::new();
    collect_refs(object, &mut out);
    out
}

/// Push every reference reachable within `object`'s own inline structure.
fn collect_refs(object: &Object, out: &mut Vec<ObjectId>) {
    match object {
        Object::Reference(id) => out.push(*id),
        Object::Array(items) => items.iter().for_each(|o| collect_refs(o, out)),
        Object::Dictionary(dict) => dict.iter().for_each(|(_, v)| collect_refs(v, out)),
        Object::Stream(stream) => stream.dict.iter().for_each(|(_, v)| collect_refs(v, out)),
        _ => {}
    }
}

/// Read the pixel width and height lopdf recorded on the built image XObject.
fn image_dimensions(image: &lopdf::Stream, page: u32) -> Result<(u32, u32)> {
    let width = image
        .dict
        .get(b"Width")
        .and_then(Object::as_i64)
        .ok()
        .and_then(|w| u32::try_from(w).ok());
    let height = image
        .dict
        .get(b"Height")
        .and_then(Object::as_i64)
        .ok()
        .and_then(|h| u32::try_from(h).ok());
    match (width, height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Ok((w, h)),
        _ => Err(Error::new(
            ErrorKind::Redaction,
            format!("page {page} image has no valid dimensions"),
        )),
    }
}
