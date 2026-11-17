//! Page reflatten: replace a whole page's content with a single redacted image,
//! or clear a page no codec could read.
//!
//! A page's redacted raster replaces everything it drew: its content and
//! resources are rewritten to draw that one image over the whole page, so
//! nothing of the original survives underneath, while every other page is left
//! untouched.

use bytes::Bytes;
use elide_core::{Error, Result};
use image::RgbImage;
use lopdf::xobject::image_from;
use lopdf::{Dictionary, Document, Object, Stream, dictionary};

use crate::text::{PageBoxes, page_boxes};

/// A scanned page as the image pipeline was given it, and as it came back.
#[derive(Debug, Clone)]
pub(crate) struct ScannedPage {
    /// 1-based page number.
    pub(crate) number: u32,
    /// The page's render, as surfaced.
    pub(crate) original: Bytes,
    /// The redacted render.
    pub(crate) redacted: Bytes,
}

/// One page reflatten: page `number`'s whole content replaced with `image`, a
/// redacted image of the page as displayed.
#[derive(Debug, Clone)]
pub(crate) struct PageReplacement {
    /// 1-based page number to reflatten.
    pub(crate) number: u32,
    /// The image XObject that replaces the page.
    image: Stream,
}

impl PageReplacement {
    /// Page `number` replaced by an encoded (PNG or JPEG) image.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if the bytes
    /// are not an image lopdf can embed.
    pub(crate) fn encoded(number: u32, image: Vec<u8>) -> Result<Self> {
        let image = image_from(image).map_err(|e| {
            Error::redaction(format!("could not build image for page {number}: {e}"))
        })?;
        Ok(Self { number, image })
    }

    /// Page `number` replaced by `pixels`, stored losslessly.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the pixels cannot be compressed.
    pub(crate) fn pixels(number: u32, pixels: RgbImage) -> Result<Self> {
        let mut image = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => i64::from(pixels.width()),
                "Height" => i64::from(pixels.height()),
                "ColorSpace" => "DeviceRGB",
                "BitsPerComponent" => 8,
            },
            pixels.into_raw(),
        );
        image
            .compress()
            .map_err(|e| Error::malformed(format!("failed to compress page {number}: {e}")))?;
        Ok(Self { number, image })
    }
}

/// Reflatten the pages of `doc` named in `replacements`: replace each page's
/// whole content with its redacted image, drawn over the page's displayed
/// area so the page keeps its size.
///
/// **Fail-closed:** a replacement naming a page that does not exist refuses
/// the whole rewrite rather than emitting a document with an unredacted page.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if a replacement could
/// not be applied.
pub(crate) fn reflatten(doc: &mut Document, replacements: &[PageReplacement]) -> Result<()> {
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
        doc.get_dictionary_mut(id)
            .map_err(|e| Error::malformed(format!("clear page {number}: {e}")))?
            .set("Resources", Dictionary::new());
    }
    Ok(())
}
