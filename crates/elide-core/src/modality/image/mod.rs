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

/// Image modality: data is [`ImageData`], locations are
/// [`ImageLocation`] regions, replacements are [`ImageReplacement`].
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Image;

impl Modality for Image {
    type Artifact = Layout;
    type Data = ImageData;
    type Location = ImageLocation;
    type Replacement = ImageReplacement;

    const NAME: &'static str = "image";
}

impl TextRecognizable for Image {
    /// The OCR text a recognizer inspects: the [`Layout`] an enricher
    /// stamped onto the call, or `""` when it is empty (an image that was
    /// never OCR'd) — a recognizer then finds nothing, rather than erroring.
    fn as_text<'a>(_data: &'a ImageData, artifact: &'a Layout) -> &'a str {
        artifact.text()
    }

    /// Resolve an OCR-text byte `range` to the region of the image it
    /// covers.
    ///
    /// Unlike the byte-based text modalities, an image location is a 2-D
    /// region, so `locate` resolves `range` immediately against the OCR
    /// word boxes in the [`Layout`] rather than deferring to a lift. Returns
    /// `None` when the range resolves to nothing (an empty layout, or out of
    /// bounds) — there is no region to address, so the caller drops the match
    /// rather than emit a placeless entity.
    fn locate(range: Range<usize>, _data: &ImageData, artifact: &Layout) -> Option<ImageLocation> {
        artifact.resolve(range)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitive::{BoundingBox, Dimensions, Point};
    use crate::recognition::{RecognizerContext, Scope};

    fn loc(x: f64, y: f64, w: f64, h: f64) -> ImageLocation {
        ImageLocation::new(BoundingBox::from_origin_size(Point::new(x, y), w, h))
    }

    #[test]
    fn as_text_is_empty_without_ocr() {
        let data = ImageData::new(bytes::Bytes::new(), Dimensions::new(10, 10));
        let scope = Scope::new();
        let ctx = RecognizerContext::<Image>::new(&scope);
        assert_eq!(Image::as_text(&data, &ctx.artifact), "");
    }

    /// A context whose artifacts carry a one-block, one-word OCR result.
    fn ocr_context(scope: &Scope) -> RecognizerContext<'_, Image> {
        let block = LayoutBlock::new(loc(0.0, 0.0, 100.0, 20.0), "Alice")
            .with_words(vec![LayoutWord::new(loc(0.0, 0.0, 100.0, 20.0), "Alice")]);
        let mut ctx = RecognizerContext::new(scope);
        ctx.artifact = Layout::new(vec![block]);
        ctx
    }

    #[test]
    fn as_text_reads_the_ocr_artifact() {
        let data = ImageData::new(bytes::Bytes::new(), Dimensions::new(10, 10));
        let scope = Scope::new();
        let ctx = ocr_context(&scope);
        assert_eq!(Image::as_text(&data, &ctx.artifact), "Alice");
    }

    #[test]
    fn locate_resolves_a_range_to_the_word_box() {
        let data = ImageData::new(bytes::Bytes::new(), Dimensions::new(10, 10));
        let scope = Scope::new();
        let ctx = ocr_context(&scope);
        // "Alice" is bytes 0..5.
        let region = Image::locate(0..5, &data, &ctx.artifact).expect("range resolves");
        assert_eq!(region.bounding_box.min.x, 0.0);
        assert_eq!(region.bounding_box.max.x, 100.0);
    }

    #[test]
    fn locate_without_ocr_is_none() {
        let data = ImageData::new(bytes::Bytes::new(), Dimensions::new(10, 10));
        let scope = Scope::new();
        let ctx = RecognizerContext::<Image>::new(&scope);
        // No OCR layout: the range can't be placed, so no location.
        assert!(Image::locate(0..5, &data, &ctx.artifact).is_none());
    }
}
