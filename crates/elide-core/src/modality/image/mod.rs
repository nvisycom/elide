//! [`Image`] modality: raster image content addressed by 2-D regions.

mod data;
mod layout;
mod location;
mod replacement;

use std::ops::Range;

pub use self::data::ImageData;
pub use self::layout::{Layout, LayoutBlock, LayoutWord};
pub use self::location::ImageLocation;
pub use self::replacement::ImageReplacement;
use super::{Modality, TextRecognizable};
use crate::primitive::{BoundingBox, Point};
use crate::recognition::RecognizerContext;

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
    /// The OCR text a recognizer inspects: the [`Layout`] an enricher
    /// stamped onto the call's artifacts, or `""` when none is present (an
    /// image that was never OCR'd) — a recognizer then finds nothing,
    /// rather than erroring.
    fn as_text<'a>(_data: &'a ImageData, ctx: &'a RecognizerContext<'_, Self>) -> &'a str {
        ctx.artifacts.get::<Layout>().map_or("", Layout::text)
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
            .get::<Layout>()
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
    use crate::primitive::Dimensions;
    use crate::recognition::Scope;

    fn loc(x: f64, y: f64, w: f64, h: f64) -> ImageLocation {
        ImageLocation::new(BoundingBox::from_origin_size(Point::new(x, y), w, h))
    }

    #[test]
    fn as_text_is_empty_without_ocr() {
        let data = ImageData::new(bytes::Bytes::new(), Dimensions::new(10, 10));
        let scope = Scope::<Image>::new();
        let ctx = RecognizerContext::new(&scope);
        assert_eq!(Image::as_text(&data, &ctx), "");
    }

    /// A context whose artifacts carry a one-block, one-word OCR result.
    fn ocr_context(scope: &Scope<Image>) -> RecognizerContext<'_, Image> {
        let block = LayoutBlock::new(loc(0.0, 0.0, 100.0, 20.0), "Alice")
            .with_words(vec![LayoutWord::new(loc(0.0, 0.0, 100.0, 20.0), "Alice")]);
        let mut ctx = RecognizerContext::new(scope);
        ctx.artifacts.insert(Layout::new(vec![block]));
        ctx
    }

    #[test]
    fn as_text_reads_the_ocr_artifact() {
        let data = ImageData::new(bytes::Bytes::new(), Dimensions::new(10, 10));
        let scope = Scope::<Image>::new();
        let ctx = ocr_context(&scope);
        assert_eq!(Image::as_text(&data, &ctx), "Alice");
    }

    #[test]
    fn locate_resolves_a_range_to_the_word_box() {
        let data = ImageData::new(bytes::Bytes::new(), Dimensions::new(10, 10));
        let scope = Scope::<Image>::new();
        let ctx = ocr_context(&scope);
        // "Alice" is bytes 0..5.
        let region = Image::locate(0..5, &data, &ctx);
        assert_eq!(region.bounding_box.min.x, 0.0);
        assert_eq!(region.bounding_box.max.x, 100.0);
    }

    #[test]
    fn locate_without_ocr_is_empty_region() {
        let data = ImageData::new(bytes::Bytes::new(), Dimensions::new(10, 10));
        let scope = Scope::<Image>::new();
        let ctx = RecognizerContext::new(&scope);
        let region = Image::locate(0..5, &data, &ctx);
        assert_eq!(region.bounding_box.area(), 0.0);
    }
}
