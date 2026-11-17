//! Page reflatten: replace a whole page's content with a single redacted image.
//!
//! A page's redacted raster replaces everything it drew: its content and
//! resources are rewritten to draw that one image over the whole page, so
//! nothing of the original survives underneath, while every other page is left
//! untouched.

use bytes::Bytes;
use elide_core::{Error, Result};
use elide_image::modality::{ImageFormat, ImageReplacement};
use elide_image::primitive::{BoundingBox, Dimensions};
use elide_image::{ImageBuffer, RasterImage};
use image::{RgbImage, load_from_memory};
use lopdf::{Document, Object, Stream, dictionary};

use crate::text::{PageBoxes, page_boxes};

/// A scanned page as the image pipeline redacted it.
#[derive(Debug, Clone)]
pub(crate) struct ScannedPage {
    /// 1-based page number.
    pub(crate) number: u32,
    /// The page's PNG render, every region redacted on it painted in.
    pub(crate) painted: Bytes,
    /// Each region of the render redacted, with how.
    pub(crate) redactions: Vec<(BoundingBox<u32>, ImageReplacement)>,
}

impl ScannedPage {
    /// The image that replaces the page: its render as redacted.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput)
    /// if the render cannot be decoded, and as [`PageReplacement::pixels`].
    pub(crate) fn replacement(&self) -> Result<PageReplacement> {
        let pixels = load_from_memory(&self.painted)
            .map_err(|e| {
                Error::malformed(format!(
                    "page {}'s render cannot be decoded: {e}",
                    self.number
                ))
            })?
            .into_rgb8();
        PageReplacement::pixels(self.number, pixels)
    }

    /// The image that replaces the page: `render`, a fresh render of it, with
    /// every redacted region painted over.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if `render`
    /// is not the size of the render the regions were found on, and as
    /// [`PageReplacement::pixels`].
    pub(crate) fn repainted(&self, render: RgbImage) -> Result<PageReplacement> {
        let size = Dimensions::new(render.width(), render.height());
        let mismatch = || {
            Error::redaction(format!(
                "page {}'s render is not the size of the one it was redacted on",
                self.number
            ))
        };
        if size != ImageBuffer::dimensions_of(&self.painted)? {
            return Err(mismatch());
        }
        let mut raster = RasterImage::from_rgb(size, render.into_raw(), ImageFormat::Png)
            .ok_or_else(mismatch)?;
        for (region, replacement) in &self.redactions {
            raster.redact(*region, replacement);
        }
        let pixels =
            RgbImage::from_raw(size.width, size.height, raster.into_rgb()).ok_or_else(mismatch)?;
        PageReplacement::pixels(self.number, pixels)
    }
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
