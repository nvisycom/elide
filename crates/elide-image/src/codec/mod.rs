//! Codec adapters: the raster format handlers (PNG, JPEG, TIFF).
//!
//! Each handler decodes to an in-memory image and redacts regions of it.
//!
//! Every format shares one handler shape stamped out by the
//! `impl_image_handler!` macro: the decoded image is held whole, streamed as
//! a single full-frame [`Chunk`], read by cropping, and redacted by
//! painting over bounding-box regions. Replacements use
//! [`ImageReplacement`]. Each pixel handler is also a
//! [`Container`](elide_codec::Container) exposing its EXIF as an `#exif`
//! sub-part.
//!
//! [`Chunk`]: elide_core::modality::Chunk
//! [`ImageReplacement`]: crate::modality::ImageReplacement

mod macros;

mod exif_handler;
#[cfg(feature = "jpeg")]
mod jpeg_handler;
#[cfg(feature = "png")]
mod png_handler;
#[cfg(feature = "tiff")]
mod tiff_handler;

pub use self::exif_handler::format as exif_format;
#[cfg(feature = "jpeg")]
pub use self::jpeg_handler::{format as jpeg_format, format_with as jpeg_format_with};
#[cfg(feature = "png")]
pub use self::png_handler::{format as png_format, format_with as png_format_with};
#[cfg(feature = "tiff")]
pub use self::tiff_handler::{format as tiff_format, format_with as tiff_format_with};

#[cfg(all(test, feature = "png"))]
mod tests {
    use elide_codec::content::ContentData;
    use elide_codec::{Handler, Loader};
    use elide_core::modality::{DataReader, DataWriter};
    use elide_core::redaction::Redactions;
    use image::{DynamicImage, GenericImageView, RgbaImage};

    use super::png_handler::PngLoader;
    use crate::ImageBuffer;
    use crate::modality::{Image, ImageLocation, ImageReplacement};
    use crate::primitive::{BoundingBox, Color, Dimensions, Point};

    /// A 4x4 solid-white PNG, encoded to bytes.
    fn white_png() -> ContentData {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            4,
            4,
            image::Rgba([255, 255, 255, 255]),
        ));
        let mut bytes = std::io::Cursor::new(Vec::new());
        img.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        ContentData::new(bytes::Bytes::from(bytes.into_inner()))
    }

    fn bbox(x: f64, y: f64, w: f64, h: f64) -> ImageLocation {
        ImageLocation::new(BoundingBox::from_origin(
            Point::new(x, y),
            Dimensions::new(w, h),
        ))
    }

    #[tokio::test]
    async fn decode_stream_reports_full_frame() {
        let mut h = PngLoader::default().decode(white_png()).await.unwrap();
        assert_eq!(h.format().as_str(), "elide.image.png");
        let chunk = h.read_next().await.unwrap().expect("one chunk");
        let dims = ImageBuffer::open(&chunk.data.bytes).unwrap().dimensions();
        assert_eq!((dims.width, dims.height), (4, 4));
        // The stream yields exactly one full-frame chunk.
        assert!(h.read_next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn read_at_crops_region() {
        let h = PngLoader::default().decode(white_png()).await.unwrap();
        let data = h
            .read_at(&bbox(1.0, 1.0, 2.0, 2.0))
            .await
            .unwrap()
            .expect("crop");
        let dims = ImageBuffer::open(&data.bytes).unwrap().dimensions();
        assert_eq!((dims.width, dims.height), (2, 2));
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
        let mut h = PngLoader::default().decode(white_png()).await.unwrap();
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

    /// The self-overlap composition: a JPEG whose PIXELS are redacted AND whose
    /// EXIF is stripped (via the `#exif` sub-part) must encode ONE image
    /// carrying both edits. This is the crux of the nested-metadata model.
    #[cfg(feature = "jpeg")]
    #[tokio::test]
    async fn pixel_redaction_and_exif_strip_compose_into_one_image() {
        use super::exif_handler::ExifLoader;
        use super::jpeg_handler::JpegLoader;
        use crate::test_util;

        // A 4x4 red JPEG carrying a GPS latitude tag.
        let original = test_util::jpeg_with_gps();

        // Decode the image handler; it is a Container exposing `#exif`.
        let mut image = JpegLoader::default()
            .decode(ContentData::new(original.clone()))
            .await
            .unwrap();

        // Redact a pixel region on the image handler.
        let mut batch: Redactions<Image> = Redactions::new();
        batch.push(
            bbox(0.0, 0.0, 2.0, 2.0),
            ImageReplacement::Block {
                color: Color::BLACK,
            },
        );
        image.write_at(batch).await.unwrap();

        // Drive the `#exif` sub-part: decode its bytes (the whole image), remove
        // the GPS field, encode the metadata-stripped image, and fold it back.
        let parts = image.as_container_mut().unwrap().parts();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].id.as_str(), "#exif");
        let mut meta = ExifLoader
            .decode(ContentData::new(parts[0].bytes.clone()))
            .await
            .unwrap();
        let mut meta_batch: elide_core::redaction::Redactions<
            elide_core::modality::metadata::Metadata,
        > = elide_core::redaction::Redactions::new();
        meta_batch.push(
            elide_core::modality::metadata::MetadataLocation::new("GPSLatitude"),
            elide_core::modality::metadata::MetadataReplacement::Removed,
        );
        DataWriter::write_at(&mut meta, meta_batch).await.unwrap();
        let stripped = Handler::encode(&meta).unwrap().to_bytes();
        image
            .as_container_mut()
            .unwrap()
            .replace_part(&elide_codec::LocalId::new("#exif"), stripped)
            .unwrap();

        // The one encode carries BOTH edits.
        let out = Handler::encode(&image).unwrap();
        let painted = image::load_from_memory(out.as_bytes()).unwrap();
        // JPEG is lossy, so assert the redacted corner is dark (not exact black)
        // and the untouched corner is still red.
        let redacted = painted.get_pixel(0, 0);
        assert!(
            redacted[0] < 60 && redacted[1] < 60 && redacted[2] < 60,
            "pixel redaction lost: {redacted:?}"
        );
        let untouched = painted.get_pixel(3, 3);
        assert!(
            untouched[0] > 150,
            "unredacted pixel changed: {untouched:?}"
        );

        // And the GPS tag is gone.
        assert!(
            !test_util::has_gps(out.as_bytes()),
            "GPS survived the composed encode"
        );
    }

    /// Without a metadata pipeline, the fallback `ExifPolicy` governs the output.
    /// The registered default (`format()`) is the privacy default and strips
    /// EXIF; `format_with(ExifPolicy::Retain)` is the opt-in that keeps it.
    #[tokio::test]
    async fn default_strips_exif_without_a_metadata_pipeline() {
        use super::png_handler::PngLoader;
        use crate::{ExifPolicy, test_util};

        let original = test_util::png_with_gps();
        assert!(
            test_util::has_gps_png(&original),
            "fixture should carry GPS"
        );

        let redact = |mut h: super::png_handler::PngHandler| async move {
            let mut batch: Redactions<Image> = Redactions::new();
            batch.push(
                bbox(0.0, 0.0, 2.0, 2.0),
                ImageReplacement::Block {
                    color: Color::BLACK,
                },
            );
            h.write_at(batch).await.unwrap();
            h.encode().unwrap()
        };

        // The registered default strips EXIF (privacy default).
        let default = PngLoader::default()
            .decode(ContentData::new(original.clone()))
            .await
            .unwrap();
        assert!(
            !test_util::has_gps_png(redact(default).await.as_bytes()),
            "default kept EXIF (should strip)"
        );

        // An explicit Retain policy keeps it.
        let retain = PngLoader::with_policy(ExifPolicy::Retain)
            .decode(ContentData::new(original.clone()))
            .await
            .unwrap();
        assert!(
            test_util::has_gps_png(redact(retain).await.as_bytes()),
            "Retain policy dropped EXIF (should keep)"
        );
    }

    /// When the `#exif` sub-part IS driven, its result wins and the fallback
    /// policy is ignored — even a `Retain` loader emits the metadata-stripped
    /// container. Locks in the scope boundary of the policy knob.
    #[tokio::test]
    async fn exif_subpart_overrides_the_fallback_policy() {
        use super::exif_handler::ExifLoader;
        use super::png_handler::PngLoader;
        use crate::{ExifPolicy, test_util};

        // A Retain loader (would preserve EXIF on the None branch)...
        let mut image = PngLoader::with_policy(ExifPolicy::Retain)
            .decode(ContentData::new(test_util::png_with_gps()))
            .await
            .unwrap();

        // ...but drive the #exif sub-part to strip GPS.
        let parts = image.as_container_mut().unwrap().parts();
        let mut meta = ExifLoader
            .decode(ContentData::new(parts[0].bytes.clone()))
            .await
            .unwrap();
        let mut meta_batch: elide_core::redaction::Redactions<
            elide_core::modality::metadata::Metadata,
        > = elide_core::redaction::Redactions::new();
        meta_batch.push(
            elide_core::modality::metadata::MetadataLocation::new("GPSLatitude"),
            elide_core::modality::metadata::MetadataReplacement::Removed,
        );
        DataWriter::write_at(&mut meta, meta_batch).await.unwrap();
        let stripped = Handler::encode(&meta).unwrap().to_bytes();
        image
            .as_container_mut()
            .unwrap()
            .replace_part(&elide_codec::LocalId::new("#exif"), stripped)
            .unwrap();

        // Despite the Retain policy, the #exif result wins: GPS is gone.
        let out = Handler::encode(&image).unwrap();
        assert!(
            !test_util::has_gps_png(out.as_bytes()),
            "Retain policy leaked past the #exif strip"
        );
    }
}
