//! Image modality: raster format handlers (PNG, JPEG, TIFF) that decode
//! to an in-memory image and redact regions of it.
//!
//! Every format shares one handler shape stamped out by
//! [`impl_image_handler!`]: the decoded image is held whole, streamed as
//! a single full-frame [`Chunk`], read by cropping, and redacted by
//! painting over bounding-box regions. Replacements use
//! [`ImageReplacement`] (blur, pixelate, block, remove).
//!
//! [`impl_image_handler!`]: macros::impl_image_handler
//! [`Chunk`]: elide_core::modality::Chunk
//! [`ImageReplacement`]: elide_core::modality::image::ImageReplacement

pub(crate) mod macros;
// Redaction painting is only reached through the format handlers; without
// any image format enabled (e.g. `pdf-render` pulling `internal_image` just
// for `encode_image`), it has no caller.
#[cfg(any(feature = "png", feature = "jpeg", feature = "tiff"))]
pub(crate) mod redact;

#[cfg(feature = "jpeg")]
mod jpeg_handler;
#[cfg(feature = "png")]
mod png_handler;
#[cfg(feature = "tiff")]
mod tiff_handler;

// `*_format` is `pub` so the parent `handler` module re-exports it as the
// crate's public contract. The macro defines each handler/loader pair in
// one file, so nothing else needs to name them.
#[cfg(feature = "jpeg")]
pub use self::jpeg_handler::format as jpeg_format;
#[cfg(feature = "png")]
pub use self::png_handler::format as png_format;
#[cfg(feature = "tiff")]
pub use self::tiff_handler::format as tiff_format;

#[cfg(all(test, feature = "png"))]
mod tests {
    use elide_core::modality::image::{Image, ImageLocation, ImageReplacement};
    use elide_core::modality::{DataReader, DataWriter};
    use elide_core::operator::Redactions;
    use elide_core::primitive::{BoundingBox, Color, Point};
    use image::{DynamicImage, GenericImageView, RgbaImage};

    use super::macros::encode_image;
    use super::png_handler::PngLoader;
    use crate::content::ContentData;
    use crate::{Handler, Loader};

    /// A 4x4 solid-white PNG, encoded to bytes.
    fn white_png() -> ContentData {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            4,
            4,
            image::Rgba([255, 255, 255, 255]),
        ));
        let bytes = encode_image(&img, image::ImageFormat::Png).unwrap();
        ContentData::new(bytes)
    }

    fn bbox(x: f64, y: f64, w: f64, h: f64) -> ImageLocation {
        ImageLocation::new(BoundingBox::from_origin_size(Point::new(x, y), w, h))
    }

    #[tokio::test]
    async fn decode_stream_reports_full_frame() {
        let mut h = PngLoader.decode(white_png()).await.unwrap();
        assert_eq!(h.format().as_str(), "elide.image.png");
        let chunk = h.read_next().await.unwrap().expect("one chunk");
        assert_eq!(chunk.data.dimensions.width, 4);
        assert_eq!(chunk.data.dimensions.height, 4);
        // The stream yields exactly one full-frame chunk.
        assert!(h.read_next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn read_at_crops_region() {
        let h = PngLoader.decode(white_png()).await.unwrap();
        let data = h
            .read_at(&bbox(1.0, 1.0, 2.0, 2.0))
            .await
            .unwrap()
            .expect("crop");
        assert_eq!((data.dimensions.width, data.dimensions.height), (2, 2));
        // An off-image region reads nothing.
        assert!(
            h.read_at(&bbox(99.0, 99.0, 2.0, 2.0))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn redact_block_paints_region_and_reencodes() {
        let mut h = PngLoader.decode(white_png()).await.unwrap();
        let mut batch: Redactions<Image> = Redactions::new();
        batch.push(
            bbox(0.0, 0.0, 2.0, 2.0),
            ImageReplacement::Block {
                color: Color::BLACK,
            },
        );
        h.write_at(batch).await.unwrap();

        // Re-encode and re-decode to confirm the paint survived the round trip.
        let out = h.encode().unwrap();
        let painted = image::load_from_memory(out.as_bytes()).unwrap();
        // Top-left corner is now black; an untouched corner stays white.
        assert_eq!(painted.get_pixel(0, 0), image::Rgba([0, 0, 0, 255]));
        assert_eq!(painted.get_pixel(3, 3), image::Rgba([255, 255, 255, 255]));
    }
}
