//! Redacting pages as images, behind the `render` feature: a page is
//! rendered with what it must not show painted over, and written back as
//! that one image, so none of its text, vector content, or images survives
//! underneath.
//!
//! A scanned page (no text layer) is redacted by the image pipeline on its
//! render ([`ScannedPage`]), and is [`Repaint`]ed over a fresh render when the
//! document's embedded images changed since; under
//! [`RasterMode::Always`](crate::primitive::RasterMode::Always) every page is
//! rendered with its detected glyphs painted over ([`Flatten`]). Either way the
//! page is written back by a [`PageRedaction`].

mod read;
mod redact;

use std::collections::{BTreeMap, BTreeSet};

use bytes::Bytes;
use elide_core::{Error, Result};
use elide_image::modality::{ImageFormat, ImageReplacement};
use elide_image::primitive::BoundingBox;
use elide_image::{ImageBuffer, RasterImage};
use lopdf::{Stream, dictionary};

pub(crate) use self::read::glyph_boxes;
use crate::pdf::reader::DeviceRect;
use crate::pdf::writer::compressed;
use crate::pdf::{Edit, Pdf, Read, Redact};

/// The pages `pages` (1-based) rendered at `scale` with the glyph boxes
/// `boxes` names for each painted over, each as the image that replaces it.
///
/// # Errors
///
/// As [`Renders`](crate::pdf::render::Renders), and
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) for a glyph box
/// that cannot be placed on its rendered page.
pub(crate) struct Flatten<'a> {
    pub(crate) pages: BTreeSet<u32>,
    pub(crate) scale: f32,
    pub(crate) boxes: &'a BTreeMap<u32, Vec<DeviceRect>>,
}

impl Read for Flatten<'_> {
    type Output = Vec<PageReplacement>;

    fn read(self, pdf: &Pdf) -> Result<Self::Output> {
        read::flatten(pdf, &self.pages, self.scale, self.boxes)
    }
}

/// Each of the `scanned` pages rendered fresh at `scale`, with the regions
/// redacted on it painted over, as the image that replaces it.
///
/// # Errors
///
/// As [`Renders`](crate::pdf::render::Renders), and
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if a fresh
/// render is not the size of the one its regions were found on.
pub(crate) struct Repaint<'a> {
    pub(crate) scanned: &'a [ScannedPage],
    pub(crate) scale: f32,
}

impl Read for Repaint<'_> {
    type Output = Vec<PageReplacement>;

    fn read(self, pdf: &Pdf) -> Result<Self::Output> {
        read::repaint(pdf, self.scanned, self.scale)
    }
}

/// Every page `pages` names replaced by its image: its whole content redrawn
/// as that one image over the page's displayed area, so the page keeps its
/// size.
///
/// **Fail-closed:** a replacement naming a page that does not exist refuses
/// the whole rewrite rather than emitting a document with an unredacted page.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if a replacement
/// could not be applied.
pub(crate) struct PageRedaction<'a> {
    pub(crate) pages: &'a [PageReplacement],
}

impl Redact for PageRedaction<'_> {
    fn redact(self, edit: &mut Edit) -> Result<()> {
        redact::reflatten(edit, self.pages)
    }
}

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
    /// As [`ImageBuffer::open`] if the render cannot be decoded, and as
    /// [`PageReplacement::pixels`].
    pub(crate) fn replacement(&self) -> Result<PageReplacement> {
        let render = ImageBuffer::open(&self.painted, ImageFormat::Png)?;
        PageReplacement::pixels(self.number, render.raster().clone())
    }

    /// The image that replaces the page: `render`, a fresh render of it, with
    /// every redacted region painted over.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if `render`
    /// is not the size of the render the regions were found on, and as
    /// [`PageReplacement::pixels`].
    pub(crate) fn repainted(&self, mut render: RasterImage) -> Result<PageReplacement> {
        if render.dimensions() != ImageBuffer::dimensions_of(&self.painted)? {
            return Err(Error::redaction(format!(
                "page {}'s render is not the size of the one it was redacted on",
                self.number
            )));
        }
        for (region, replacement) in &self.redactions {
            render.redact(*region, replacement);
        }
        PageReplacement::pixels(self.number, render)
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
    pub(crate) fn pixels(number: u32, pixels: RasterImage) -> Result<Self> {
        let size = pixels.dimensions();
        let image = compressed(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => i64::from(size.width),
                "Height" => i64::from(size.height),
                "ColorSpace" => "DeviceRGB",
                "BitsPerComponent" => 8,
            },
            pixels.into_rgb(),
        )?;
        Ok(Self { number, image })
    }
}
