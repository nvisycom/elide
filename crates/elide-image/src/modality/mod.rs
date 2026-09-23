//! [`Image`] modality: raster image content addressed by 2-D regions.

mod data;
mod format;
mod layout;
mod location;
mod replacement;

use std::ops::Range;

use elide_core::modality::{Modality, TextRecognizable};

pub use self::data::ImageData;
pub use self::format::ImageFormat;
pub use self::layout::{Layout, LayoutBlock, LayoutWord};
pub use self::location::ImageLocation;
pub use self::replacement::ImageReplacement;

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
    /// The OCR text a recognizer inspects: the [`Layout`] an enricher stamped
    /// onto the call, or [`None`] when the image was never OCR'd (no artifact),
    /// so a recognizer skips it rather than scanning an empty string.
    fn as_text<'a>(_data: &'a ImageData, artifact: Option<&'a Layout>) -> Option<&'a str> {
        artifact.map(Layout::text)
    }

    /// Resolve an OCR-text byte `range` to the region of the image it
    /// covers.
    ///
    /// Unlike the byte-based text modalities, an image location is a 2-D
    /// region, so `locate` resolves `range` immediately against the OCR
    /// word boxes in the [`Layout`] rather than deferring to a lift. Returns
    /// `None` when the range resolves to nothing (an empty layout, or out of
    /// bounds), there is no region to address, so the caller drops the match
    /// rather than emit a placeless entity.
    fn locate(
        range: Range<usize>,
        _data: &ImageData,
        artifact: Option<&Layout>,
    ) -> Option<ImageLocation> {
        artifact?.resolve(range)
    }
}

#[cfg(test)]
mod tests {
    use elide_core::recognition::Subject;

    use super::*;
    use crate::primitive::{BoundingBox, Dimensions, Point};

    fn loc(x: f64, y: f64, w: f64, h: f64) -> ImageLocation {
        ImageLocation::new(BoundingBox::from_origin(
            Point::new(x, y),
            Dimensions::new(w, h),
        ))
    }

    #[test]
    fn as_text_is_none_without_ocr() {
        let subject = Subject::<Image>::new(ImageData::new(bytes::Bytes::new()));
        assert_eq!(Image::as_text(subject.data(), subject.artifact()), None);
    }

    /// A subject whose artifact carries a one-block, one-word OCR result.
    fn ocr_subject() -> Subject<Image> {
        let block = LayoutBlock::new(loc(0.0, 0.0, 100.0, 20.0), "Alice")
            .with_words(vec![LayoutWord::new(loc(0.0, 0.0, 100.0, 20.0), "Alice")]);
        Subject::new(ImageData::new(bytes::Bytes::new())).with_artifact(Layout::new(vec![block]))
    }

    #[test]
    fn as_text_reads_the_ocr_artifact() {
        let subject = ocr_subject();
        assert_eq!(
            Image::as_text(subject.data(), subject.artifact()),
            Some("Alice")
        );
    }

    #[test]
    fn locate_resolves_a_range_to_the_word_box() {
        let subject = ocr_subject();
        // "Alice" is bytes 0..5.
        let region =
            Image::locate(0..5, subject.data(), subject.artifact()).expect("range resolves");
        assert_eq!(region.bounding_box.min.x, 0.0);
        assert_eq!(region.bounding_box.max.x, 100.0);
    }

    #[test]
    fn locate_without_ocr_is_none() {
        let subject = Subject::<Image>::new(ImageData::new(bytes::Bytes::new()));
        // No OCR layout: the range can't be placed, so no location.
        assert!(Image::locate(0..5, subject.data(), subject.artifact()).is_none());
    }
}
