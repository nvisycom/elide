//! Writing a page's image back: its whole content replaced by that one
//! image, so nothing of the original survives underneath.

use elide_core::{Error, Result};

use super::PageReplacement;
use crate::pdf::Edit;
use crate::pdf::object::{Value, dict};
use crate::pdf::reader::{PageBoxes, page_boxes};
use crate::pdf::writer::{Changes, compressed, dict_value};

/// Replace the whole content of each page `replacements` names with its
/// image; see [`PageRedaction`](super::PageRedaction).
pub(super) fn reflatten(edit: &mut Edit, replacements: &[PageReplacement]) -> Result<()> {
    let pdf = edit.pdf();
    let pages = pdf.pages();

    // The replaced pages' old content and resource objects are no longer
    // named once each page is repointed, so they are not written and the
    // original page does not survive in the bytes. An object shared with a
    // surviving page is still named, and kept.
    let mut changes = Changes::new();
    for replacement in replacements {
        let number = replacement.number;
        let Some(page) = usize::try_from(number)
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|i| pages.get(i))
        else {
            return Err(Error::redaction(format!("page {number} does not exist")));
        };
        let page_id = page
            .obj_id()
            .ok_or_else(|| Error::redaction(format!("page {number} is unreadable")))?;
        // The image is the page as displayed (cropped, rotated, at its user
        // unit), so it is drawn over a page of that size with no crop,
        // rotation, or unit of its own. A page with no box renders as US
        // Letter.
        let (width, height) = page_boxes(page)
            .unwrap_or(PageBoxes::LETTER)
            .displayed_size();

        let image = changes.add(replacement.image.clone());
        let content = format!("q {width} 0 0 {height} 0 0 cm /Im0 Do Q");
        let content = changes.add(compressed(dict! {}, content.as_bytes())?);

        let mut dict = dict_value(page);
        dict.set("Contents", content.value());
        // Held in place, as the normal form has it.
        dict.set(
            "Resources",
            dict! { "XObject" => dict! { "Im0" => image.value() } },
        );
        // Each is set on the page itself, so an inherited value from an
        // ancestor `Pages` node cannot clip, rotate, or rescale the image.
        let size: Vec<Value> = vec![0.into(), 0.into(), width.into(), height.into()];
        dict.set("MediaBox", size.clone());
        dict.set("CropBox", size);
        dict.set("Rotate", 0);
        dict.set("UserUnit", 1);
        changes.replace(page_id, dict);
    }
    edit.apply(changes)
}
