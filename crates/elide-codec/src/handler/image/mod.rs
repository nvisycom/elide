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

#[cfg(any(feature = "png", feature = "jpeg", feature = "tiff"))]
mod exif_handler;
#[cfg(feature = "jpeg")]
mod jpeg_handler;
#[cfg(feature = "png")]
mod png_handler;
#[cfg(feature = "tiff")]
mod tiff_handler;

// `*_format` is `pub` so the parent `handler` module re-exports it as the
// crate's public contract. The macro defines each handler/loader pair in
// one file, so nothing else needs to name them.
#[cfg(any(feature = "png", feature = "jpeg", feature = "tiff"))]
pub use self::exif_handler::format as exif_format;
#[cfg(feature = "jpeg")]
pub use self::jpeg_handler::{format as jpeg_format, format_with as jpeg_format_with};
#[cfg(feature = "png")]
pub use self::png_handler::{format as png_format, format_with as png_format_with};
#[cfg(feature = "tiff")]
pub use self::tiff_handler::{format as tiff_format, format_with as tiff_format_with};

#[cfg(all(test, feature = "png"))]
mod tests {
    use elide_core::modality::image::{Image, ImageLocation, ImageReplacement};
    use elide_core::modality::{DataReader, DataWriter};
    use elide_core::primitive::{BoundingBox, Color, Point};
    use elide_core::redaction::Redactions;
    use image::{DynamicImage, GenericImageView, RgbaImage};

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
        let mut bytes = std::io::Cursor::new(Vec::new());
        img.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        ContentData::new(bytes::Bytes::from(bytes.into_inner()))
    }

    fn bbox(x: f64, y: f64, w: f64, h: f64) -> ImageLocation {
        ImageLocation::new(BoundingBox::from_origin_size(Point::new(x, y), w, h))
    }

    #[tokio::test]
    async fn decode_stream_reports_full_frame() {
        let mut h = PngLoader::default().decode(white_png()).await.unwrap();
        assert_eq!(h.format().as_str(), "elide.image.png");
        let chunk = h.read_next().await.unwrap().expect("one chunk");
        assert_eq!(chunk.data.dimensions.width, 4);
        assert_eq!(chunk.data.dimensions.height, 4);
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
        use little_exif::exif_tag::ExifTag;
        use little_exif::filetype::FileExtension;
        use little_exif::metadata::Metadata as ExifMetadata;

        use super::exif_handler::ExifLoader;
        use super::jpeg_handler::JpegLoader;

        // A 4x4 red JPEG carrying a GPS latitude tag.
        let mut bytes = Vec::new();
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 4, image::Rgb([200, 30, 30])))
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        let mut exif = ExifMetadata::new();
        exif.set_tag(ExifTag::GPSLatitude(vec![little_exif::rational::uR64 {
            nominator: 51,
            denominator: 1,
        }]));
        exif.write_to_vec(&mut bytes, FileExtension::JPEG).unwrap();
        let original = bytes::Bytes::from(bytes);

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
            .replace_part(&crate::LocalId::new("#exif"), stripped)
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
        let readback = ExifMetadata::new_from_vec(&out.as_bytes().to_vec(), FileExtension::JPEG);
        let has_gps = readback
            .ok()
            .map(|m| {
                m.get_tag(&ExifTag::GPSLatitude(Vec::new()))
                    .next()
                    .is_some()
            })
            .unwrap_or(false);
        assert!(!has_gps, "GPS survived the composed encode");
    }

    /// A 4x4 red PNG carrying a GPS latitude EXIF tag.
    fn png_with_gps() -> bytes::Bytes {
        use little_exif::exif_tag::ExifTag;
        use little_exif::filetype::FileExtension;
        use little_exif::metadata::Metadata as ExifMetadata;

        let mut bytes = Vec::new();
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(4, 4, image::Rgb([200, 30, 30])))
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        let mut exif = ExifMetadata::new();
        exif.set_tag(ExifTag::GPSLatitude(vec![little_exif::rational::uR64 {
            nominator: 51,
            denominator: 1,
        }]));
        exif.write_to_vec(
            &mut bytes,
            FileExtension::PNG {
                as_zTXt_chunk: false,
            },
        )
        .unwrap();
        bytes::Bytes::from(bytes)
    }

    fn png_has_gps(png: &[u8]) -> bool {
        use little_exif::exif_tag::ExifTag;
        use little_exif::filetype::FileExtension;
        use little_exif::metadata::Metadata as ExifMetadata;

        ExifMetadata::new_from_vec(
            &png.to_vec(),
            FileExtension::PNG {
                as_zTXt_chunk: false,
            },
        )
        .map(|m| {
            m.get_tag(&ExifTag::GPSLatitude(Vec::new()))
                .next()
                .is_some()
        })
        .unwrap_or(false)
    }

    /// `format_with(StripAll)` strips a redacted image's EXIF when NO metadata
    /// pipeline ran (the `#exif` sub-part was never driven), while the default
    /// `format()` keeps it.
    #[tokio::test]
    async fn format_with_strip_all_drops_exif_without_a_metadata_pipeline() {
        use elide_image::ExifPolicy;

        use super::png_handler::PngLoader;

        let original = png_with_gps();
        assert!(png_has_gps(&original), "fixture should carry GPS");

        // A loader carrying StripAll — mirrors png_format_with(ExifPolicy::StripAll).
        let mut h = PngLoader::with_policy(ExifPolicy::StripAll)
            .decode(ContentData::new(original.clone()))
            .await
            .unwrap();
        // Redact a pixel so encode takes the re-encode path.
        let mut batch: Redactions<Image> = Redactions::new();
        batch.push(
            bbox(0.0, 0.0, 2.0, 2.0),
            ImageReplacement::Block {
                color: Color::BLACK,
            },
        );
        h.write_at(batch).await.unwrap();
        let out = h.encode().unwrap();
        assert!(!png_has_gps(out.as_bytes()), "StripAll fallback kept EXIF");

        // The default keeps it.
        let mut keep = PngLoader::default()
            .decode(ContentData::new(original.clone()))
            .await
            .unwrap();
        let mut batch: Redactions<Image> = Redactions::new();
        batch.push(
            bbox(0.0, 0.0, 2.0, 2.0),
            ImageReplacement::Block {
                color: Color::BLACK,
            },
        );
        keep.write_at(batch).await.unwrap();
        assert!(
            png_has_gps(keep.encode().unwrap().as_bytes()),
            "default dropped EXIF"
        );
    }

    /// When the `#exif` sub-part IS driven, its result wins and the fallback
    /// policy is ignored — even a `Keep` loader emits the metadata-stripped
    /// container. Locks in the scope boundary of the policy knob.
    #[tokio::test]
    async fn exif_subpart_overrides_the_fallback_policy() {
        use elide_image::ExifPolicy;

        use super::exif_handler::ExifLoader;
        use super::png_handler::PngLoader;

        // A Keep loader (would preserve EXIF on the None branch)...
        let mut image = PngLoader::with_policy(ExifPolicy::Keep)
            .decode(ContentData::new(png_with_gps()))
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
            .replace_part(&crate::LocalId::new("#exif"), stripped)
            .unwrap();

        // Despite the Keep policy, the #exif result wins: GPS is gone.
        let out = Handler::encode(&image).unwrap();
        assert!(
            !png_has_gps(out.as_bytes()),
            "Keep policy leaked past the #exif strip"
        );
    }
}
