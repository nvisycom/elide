//! Small in-memory image fixtures for exercising the EXIF path, behind the
//! `test-util` feature.
//!
//! A downstream crate testing metadata redaction needs a real image carrying
//! real EXIF, and a way to check whether a tag survived a round-trip, without
//! taking a direct dependency on `image` or `little_exif`. These helpers build
//! and inspect such fixtures so the test stays in this crate's vocabulary.

use bytes::Bytes;
use little_exif::exif_tag::ExifTag;
use little_exif::filetype::FileExtension;
use little_exif::metadata::Metadata as ExifMetadata;
use little_exif::rational::uR64;

/// A tiny solid-colour JPEG carrying a GPS latitude EXIF tag.
///
/// The smallest fixture that exercises the full metadata path: a real JPEG a
/// codec decodes, with one privacy-relevant tag a recognizer surfaces and a
/// strip removes.
#[must_use]
pub fn jpeg_with_gps() -> Bytes {
    let mut bytes = Vec::new();
    let image = image::RgbImage::from_pixel(4, 4, image::Rgb([200, 30, 30]));
    image::DynamicImage::ImageRgb8(image)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Jpeg,
        )
        .expect("encode jpeg fixture");
    let mut exif = ExifMetadata::new();
    exif.set_tag(ExifTag::GPSLatitude(vec![uR64 {
        nominator: 51,
        denominator: 1,
    }]));
    exif.write_to_vec(&mut bytes, FileExtension::JPEG)
        .expect("write exif fixture");
    Bytes::from(bytes)
}

/// Whether `jpeg` bytes still carry a GPS latitude tag, for asserting a strip
/// took effect.
#[must_use]
pub fn has_gps(jpeg: &[u8]) -> bool {
    ExifMetadata::new_from_vec(&jpeg.to_vec(), FileExtension::JPEG)
        .map(|meta| {
            meta.get_tag(&ExifTag::GPSLatitude(Vec::new()))
                .next()
                .is_some()
        })
        .unwrap_or(false)
}

/// Whether `bytes` decode as a valid image.
#[must_use]
pub fn is_valid_image(bytes: &[u8]) -> bool {
    image::load_from_memory(bytes).is_ok()
}
