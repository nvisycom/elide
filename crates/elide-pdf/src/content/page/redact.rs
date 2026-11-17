//! Writing a page's image back: its whole content replaced by that one
//! image, so nothing of the original survives underneath.

use elide_core::{Error, Result};
use lopdf::{Object, Stream, dictionary};

use super::PageReplacement;
use crate::pdf::Edit;
use crate::pdf::reader::{PageBoxes, page_boxes};

/// Replace the whole content of each page `replacements` names with its
/// image; see [`PageRedaction`](super::PageRedaction).
pub(super) fn reflatten(edit: &mut Edit, replacements: &[PageReplacement]) -> Result<()> {
    let doc = edit.doc_mut();
    let pages = doc.get_pages();

    // The replaced pages' old content and resource objects become unreachable
    // once each page is repointed, and the save prunes them, so the original
    // page does not survive in the bytes. An object shared with a surviving
    // page stays reachable and is kept.
    for replacement in replacements {
        let number = replacement.number;
        let Some(&page_id) = pages.get(&number) else {
            return Err(Error::redaction(format!("page {number} does not exist")));
        };
        // The image is the page as displayed (cropped, rotated, at its user
        // unit), so it is drawn over a page of that size with no crop,
        // rotation, or unit of its own. A page with no box renders as US
        // Letter.
        let (width, height) = page_boxes(doc, page_id)
            .unwrap_or(PageBoxes::LETTER)
            .displayed_size();

        let image_id = doc.add_object(replacement.image.clone());
        let content = format!("q {width} 0 0 {height} 0 0 cm /Im0 Do Q");
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let resources_id = doc.add_object(dictionary! {
            "XObject" => dictionary! { "Im0" => Object::Reference(image_id) },
        });

        let page = doc
            .get_dictionary_mut(page_id)
            .map_err(|e| Error::redaction(format!("page {number} is unreadable: {e}")))?;
        page.set("Contents", Object::Reference(content_id));
        page.set("Resources", Object::Reference(resources_id));
        // Each is set on the page itself, so an inherited value from an
        // ancestor `Pages` node cannot clip, rotate, or rescale the image.
        let size = vec![
            0.into(),
            0.into(),
            Object::Real(width as f32),
            Object::Real(height as f32),
        ];
        page.set("MediaBox", size.clone());
        page.set("CropBox", size);
        page.set("Rotate", 0);
        page.set("UserUnit", 1);
    }
    Ok(())
}
