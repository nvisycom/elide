//! Page reflatten: replace a whole page's content with a single redacted image,
//! or clear a page no codec could read.
//!
//! A scanned or image-only page carries no born-digital text to delete, so its
//! redaction is a redacted raster of the page. A caller renders the page,
//! redacts the pixels, and hands the encoded image back as a
//! [`PageReplacement`]; the page's content and resources are then rewritten to
//! draw that one image over the whole page, so nothing of the original page
//! content survives underneath, while every other page of the document is left
//! untouched (a born-digital page keeps its selectable, glyph-redacted text).

use elide_core::{Error, Result};
use lopdf::{Dictionary, Document, Object, dictionary};

/// One page reflatten: replace page [`number`](PageReplacement::number)'s whole
/// content with a redacted image of the page.
///
/// The `image` bytes are a self-contained encoded image (PNG or JPEG) of the
/// redacted page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageReplacement {
    /// 1-based page number to reflatten.
    pub number: u32,
    /// The redacted page image, encoded as PNG or JPEG.
    pub image: Vec<u8>,
}

/// Reflatten the pages of `doc` named in `replacements`: replace each page's
/// whole content with a single redacted image.
///
/// Only the named pages are rewritten, so a mixed document keeps its
/// born-digital pages' selectable text and flattens only the scanned ones.
///
/// **Fail-closed:** a replacement naming a page that does not exist, or an image
/// that cannot be decoded into a valid XObject, refuses the whole rewrite rather
/// than emitting a document with an unredacted page.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if a replacement could
/// not be applied.
pub(crate) fn reflatten(doc: &mut Document, replacements: &[PageReplacement]) -> Result<()> {
    let pages = doc.get_pages();

    // The replaced pages' old content and resource objects become unreachable
    // once each page is repointed, and the save prunes them, so the original
    // page raster does not survive in the bytes. An object shared with a
    // surviving page stays reachable and is kept.
    for replacement in replacements {
        let Some(&page_id) = pages.get(&replacement.number) else {
            return Err(Error::redaction(format!(
                "page {} does not exist",
                replacement.number
            )));
        };

        // Build a valid image XObject from the encoded bytes; lopdf sets the
        // dimensions, colour space, and filter to match.
        let image = lopdf::xobject::image_from(replacement.image.clone()).map_err(|e| {
            Error::redaction(format!(
                "could not build image for page {}: {e}",
                replacement.number
            ))
        })?;
        let (width, height) = image_dimensions(&image, replacement.number)?;
        let image_id = doc.add_object(image);

        // Draw the image to fill the page: `q W 0 0 H cm /Im0 Do Q`, with the
        // page sized one unit per pixel so the image maps 1:1.
        let content = format!("q {width} 0 0 {height} 0 0 cm /Im0 Do Q");
        let content_id = doc.add_object(lopdf::Stream::new(dictionary! {}, content.into_bytes()));
        let resources_id = doc.add_object(dictionary! {
            "XObject" => dictionary! { "Im0" => Object::Reference(image_id) },
        });

        // Repoint the page at the new content and resources, and size its
        // box to the image.
        let page = doc.get_dictionary_mut(page_id).map_err(|e| {
            Error::redaction(format!("page {} is unreadable: {e}", replacement.number))
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
    Ok(())
}

/// Clear the content of the 1-based pages `numbers` of `doc`: each page keeps
/// its place and size, but draws nothing and holds no resources, so what it
/// drew is dropped from the file.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a page does not exist or its content
/// cannot be replaced.
pub(crate) fn clear(doc: &mut Document, numbers: &[u32]) -> Result<()> {
    let pages = doc.get_pages();
    for number in numbers {
        let id = *pages
            .get(number)
            .ok_or_else(|| Error::malformed(format!("page {number} does not exist")))?;
        doc.change_page_content(id, Vec::new())
            .map_err(|e| Error::malformed(format!("clear page {number}: {e}")))?;
        if let Ok(page) = doc.get_dictionary_mut(id) {
            page.set("Resources", Dictionary::new());
        }
    }
    Ok(())
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
        _ => Err(Error::redaction(format!(
            "page {page} image has no valid dimensions"
        ))),
    }
}
