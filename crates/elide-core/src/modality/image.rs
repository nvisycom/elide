//! [`Image`] modality: raster image content addressed by 2-D regions.

use std::cmp::Ordering;
use std::ops::Range;

use bytes::Bytes;
use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::{Modality, ModalityData, ModalityLocation, ModalityReplacement, TextRecognizable};
use crate::primitive::{BoundingBox, Color, Dimensions, OcrText, Point, Polygon};
use crate::recognition::RecognizerContext;

/// Per-call payload a recognizer inspects for the [`Image`] modality.
///
/// Carries the encoded bytes plus the pixel [`Dimensions`], which a
/// recognizer that emits unit-square boxes needs to scale them into pixel
/// coordinates. An optional filename aids diagnostics and encoding
/// inference.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ImageData {
    /// Encoded image bytes. Skipped by serde: the bytes are the raw payload,
    /// not metadata, and a serialized report (entities, provenance) has no
    /// need to carry megabytes of image data.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub bytes: Bytes,
    /// Pixel dimensions of the encoded image.
    pub dimensions: Dimensions,
    /// Original filename, when known.
    pub filename: Option<HipStr<'static>>,
}

impl ImageData {
    /// Wrap encoded bytes and their pixel dimensions; filename unset.
    pub fn new(bytes: impl Into<Bytes>, dimensions: Dimensions) -> Self {
        Self {
            bytes: bytes.into(),
            dimensions,
            filename: None,
        }
    }

    /// Attach an original filename.
    #[must_use]
    pub fn with_filename(mut self, filename: impl Into<HipStr<'static>>) -> Self {
        self.filename = Some(filename.into());
        self
    }

    /// Lowercased extension derived from [`filename`](Self::filename),
    /// or `"png"` when no filename is set or it has no extension.
    pub fn extension(&self) -> &str {
        self.filename
            .as_deref()
            .and_then(|name| name.rsplit_once('.'))
            .map(|(_, ext)| ext)
            .unwrap_or("png")
    }
}

impl ModalityData for ImageData {}

/// Region within image content.
///
/// An axis-aligned [`BoundingBox`] in pixel coordinates locates the
/// region; an optional [`Polygon`] captures a rotated or quadrilateral
/// shape when the source produced one (OCR engines that emit 4-point
/// polygons), and an optional page number addresses multi-page documents.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ImageLocation {
    /// Axis-aligned bounding box of the region, in pixel coordinates.
    pub bounding_box: BoundingBox,
    /// Polygon vertices when the region is rotated or quadrilateral.
    /// Axis-aligned-only sources leave this unset.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub polygon: Option<Polygon>,
    /// 1-based page number, for multi-page documents like PDFs.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub page: Option<u32>,
}

impl ImageLocation {
    /// Location from the bounding box alone, every optional field unset.
    pub fn new(bounding_box: BoundingBox) -> Self {
        Self {
            bounding_box,
            polygon: None,
            page: None,
        }
    }

    /// Set the page number.
    #[must_use]
    pub fn with_page(mut self, page: u32) -> Self {
        self.page = Some(page);
        self
    }

    /// Set the region polygon.
    #[must_use]
    pub fn with_polygon(mut self, polygon: Polygon) -> Self {
        self.polygon = Some(polygon);
        self
    }
}

impl ImageLocation {
    /// Region's shape as a polygon: its explicit [`polygon`] when set,
    /// otherwise its bounding box as a rectangle.
    ///
    /// [`polygon`]: Self::polygon
    fn shape(&self) -> Polygon {
        self.polygon
            .clone()
            .unwrap_or_else(|| self.bounding_box.to_polygon())
    }
}

impl ModalityLocation for ImageLocation {
    fn overlaps(&self, other: &Self) -> bool {
        // Regions on different pages never overlap, even with identical
        // coordinates. On the same page, compare exact shapes: a rotated
        // or quadrilateral polygon is honored when present, so two boxes
        // whose polygons don't actually intersect aren't false positives.
        if self.page != other.page {
            return false;
        }
        // Bounding-box test first as a cheap reject; polygons only narrow
        // it, so a box miss is a definite miss.
        self.bounding_box.overlaps(&other.bounding_box) && self.shape().overlaps(&other.shape())
    }

    fn span_cmp(&self, other: &Self) -> Ordering {
        // By area: the larger region is the more specific match.
        self.bounding_box
            .area()
            .total_cmp(&other.bounding_box.area())
    }

    fn position_cmp(&self, other: &Self) -> Ordering {
        // Reading order: page, then top-to-bottom, then left-to-right.
        self.page
            .unwrap_or(0)
            .cmp(&other.page.unwrap_or(0))
            .then(self.bounding_box.min.y.total_cmp(&other.bounding_box.min.y))
            .then(self.bounding_box.min.x.total_cmp(&other.bounding_box.min.x))
    }
}

/// What an image operator produces to hide an entity: a visual treatment
/// applied to the entity's region.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum ImageReplacement {
    /// Gaussian blur over the region.
    Blur {
        /// Standard deviation of the Gaussian kernel, in pixels.
        sigma: f32,
    },
    /// Mosaic pixelation over the region.
    Pixelate {
        /// Side length of each mosaic block, in pixels.
        block_size: u32,
    },
    /// Solid-color block over the region.
    Block {
        /// Fill color the codec rasterizes over the region.
        color: Color,
    },
    /// Remove the region entirely (cut or fully obscure).
    Removed,
}

impl ImageReplacement {
    /// Black block, the conservative default treatment.
    pub const fn block() -> Self {
        Self::Block {
            color: Color::BLACK,
        }
    }
}

impl ModalityReplacement for ImageReplacement {}

/// Image modality: data is [`ImageData`], locations are
/// [`ImageLocation`] regions, replacements are [`ImageReplacement`].
#[derive(Debug, Clone, Copy)]
pub struct Image;

impl Modality for Image {
    type Data = ImageData;
    type Location = ImageLocation;
    type Replacement = ImageReplacement;

    const NAME: &'static str = "image";
}

impl TextRecognizable for Image {
    /// The OCR text a recognizer inspects: the [`OcrText`] an enricher
    /// stamped onto the call's artifacts, or `""` when none is present (an
    /// image that was never OCR'd) — a recognizer then finds nothing,
    /// rather than erroring.
    fn as_text<'a>(_data: &'a ImageData, ctx: &'a RecognizerContext<'_, Self>) -> &'a str {
        ctx.artifacts.get::<OcrText>().map_or("", OcrText::text)
    }

    /// Resolve an OCR-text byte `range` to the region of the image it
    /// covers.
    ///
    /// Unlike the byte-based text modalities, an image location is a 2-D
    /// region, so `locate` resolves `range` immediately against the OCR
    /// word boxes (read from the call's artifacts) rather than deferring to
    /// a lift. A range that resolves to nothing (no OCR, or out of bounds)
    /// yields an empty region at the origin; such an entity carries no real
    /// image extent.
    fn locate(
        range: Range<usize>,
        _data: &ImageData,
        ctx: &RecognizerContext<'_, Self>,
    ) -> ImageLocation {
        ctx.artifacts
            .get::<OcrText>()
            .and_then(|t| t.resolve(range))
            .unwrap_or_else(|| {
                // No OCR / out of bounds: an empty region at the origin,
                // carrying no real image extent.
                ImageLocation::new(BoundingBox::new(Point::new(0.0, 0.0), Point::new(0.0, 0.0)))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitive::{OcrBlock, OcrWord, Point};
    use crate::recognition::Scope;

    fn loc(x: f64, y: f64, w: f64, h: f64) -> ImageLocation {
        ImageLocation::new(BoundingBox::from_origin_size(Point::new(x, y), w, h))
    }

    #[test]
    fn overlaps_requires_same_page() {
        let a = loc(0.0, 0.0, 10.0, 10.0);
        let b = loc(5.0, 5.0, 10.0, 10.0);
        assert!(a.overlaps(&b));
        let b_page2 = b.clone().with_page(2);
        assert!(!a.overlaps(&b_page2));
    }

    #[test]
    fn span_cmp_orders_by_area() {
        let small = loc(0.0, 0.0, 2.0, 2.0);
        let large = loc(0.0, 0.0, 10.0, 10.0);
        assert_eq!(small.span_cmp(&large), Ordering::Less);
    }

    #[test]
    fn position_cmp_is_reading_order() {
        let top = loc(50.0, 0.0, 5.0, 5.0);
        let bottom_left = loc(0.0, 100.0, 5.0, 5.0);
        // Top row sorts before a lower row regardless of x.
        assert_eq!(top.position_cmp(&bottom_left), Ordering::Less);
    }

    #[test]
    fn extension_falls_back_to_png() {
        let d = ImageData::new(Bytes::new(), Dimensions::new(10, 10));
        assert_eq!(d.extension(), "png");
        let named = d.with_filename("scan.JPEG");
        assert_eq!(named.extension(), "JPEG");
    }

    #[test]
    fn as_text_is_empty_without_ocr() {
        let data = ImageData::new(Bytes::new(), Dimensions::new(10, 10));
        let scope = Scope::<Image>::new();
        let ctx = RecognizerContext::new(&scope);
        assert_eq!(Image::as_text(&data, &ctx), "");
    }

    /// A context whose artifacts carry a one-block, one-word OCR result.
    fn ocr_context(scope: &Scope<Image>) -> RecognizerContext<'_, Image> {
        let block =
            OcrBlock::new(loc(0.0, 0.0, 100.0, 20.0), "Alice").with_words(vec![OcrWord::new(
                loc(0.0, 0.0, 100.0, 20.0),
                "Alice",
            )]);
        let mut ctx = RecognizerContext::new(scope);
        ctx.artifacts.insert(OcrText::new(vec![block]));
        ctx
    }

    #[test]
    fn as_text_reads_the_ocr_artifact() {
        let data = ImageData::new(Bytes::new(), Dimensions::new(10, 10));
        let scope = Scope::<Image>::new();
        let ctx = ocr_context(&scope);
        assert_eq!(Image::as_text(&data, &ctx), "Alice");
    }

    #[test]
    fn locate_resolves_a_range_to_the_word_box() {
        let data = ImageData::new(Bytes::new(), Dimensions::new(10, 10));
        let scope = Scope::<Image>::new();
        let ctx = ocr_context(&scope);
        // "Alice" is bytes 0..5.
        let region = Image::locate(0..5, &data, &ctx);
        assert_eq!(region.bounding_box.min.x, 0.0);
        assert_eq!(region.bounding_box.max.x, 100.0);
    }

    #[test]
    fn locate_without_ocr_is_empty_region() {
        let data = ImageData::new(Bytes::new(), Dimensions::new(10, 10));
        let scope = Scope::<Image>::new();
        let ctx = RecognizerContext::new(&scope);
        let region = Image::locate(0..5, &data, &ctx);
        assert_eq!(region.bounding_box.area(), 0.0);
    }
}
