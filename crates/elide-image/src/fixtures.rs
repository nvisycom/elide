//! Small in-memory image fixtures, behind the `fixtures` feature.
//!
//! A downstream crate testing metadata redaction needs a real image carrying
//! real EXIF, and a way to check whether a tag survived a round-trip, without
//! taking a direct dependency on `image` or `little_exif`. These helpers build
//! and inspect such fixtures so the test stays in this crate's vocabulary.

use std::io::Cursor;

use bytes::Bytes;
use image::GenericImageView;
use little_exif::exif_tag::ExifTag;
use little_exif::filetype::FileExtension;
use little_exif::metadata::Metadata as ExifMetadata;
use little_exif::rational::uR64;

use crate::ImageBuffer;
use crate::modality::{ImageData, ImageFormat};

/// Decode fixture `bytes` of a known `format` into an [`ImageData`] payload.
///
/// The way ingestion does, for a test that needs the image modality's payload
/// from raw container bytes.
///
/// # Panics
///
/// Panics if `bytes` is not a decodable image of `format`; a test wanting a
/// placeholder payload should pass a real fixture (e.g. [`png`]).
#[must_use]
pub fn image_data(bytes: impl Into<Bytes>, format: ImageFormat) -> ImageData {
    let bytes = bytes.into();
    let buffer = ImageBuffer::open(&bytes, format).expect("decodable image fixture");
    ImageData::new(buffer.raster().clone(), bytes)
}

/// A tiny (1x1) decoded [`ImageData`], for a test that needs an image.
///
/// For a test that needs *some* valid image payload but does not care about its
/// pixels (it exercises the OCR artifact or an empty-recognition path, not the
/// image itself).
#[must_use]
pub fn blank_image_data() -> ImageData {
    image_data(png(1, 1), ImageFormat::Png)
}

/// A plain solid-colour PNG of `width`x`height` pixels, no metadata.
///
/// For a downstream test that needs a real, decodable image of a *known size*
/// (e.g. to check a normalized box scales to the right pixels) without taking a
/// direct dependency on the `image` crate.
#[must_use]
pub fn png(width: u32, height: u32) -> Bytes {
    Bytes::from(solid(width, height, image::ImageFormat::Png))
}

/// A tiny solid-colour JPEG carrying a GPS latitude EXIF tag.
///
/// The smallest fixture that exercises the full metadata path: a real JPEG a
/// codec decodes, with one privacy-relevant tag a recognizer surfaces and a
/// strip removes.
#[must_use]
pub fn jpeg_with_gps() -> Bytes {
    jpeg_with_tags(true, &[])
}

/// A tiny solid-colour JPEG whose EXIF `ImageDescription` is `description`.
#[must_use]
pub fn jpeg_with_description(description: &str) -> Bytes {
    jpeg_with_tags(false, &[("ImageDescription", description)])
}

/// A tiny solid-colour JPEG carrying the EXIF text `tags`, each a tag name
/// (`Artist`, `Make`, `Model`, or `ImageDescription`) and its value, plus a
/// GPS latitude when `gps`.
///
/// # Panics
///
/// Panics on a tag name other than those four.
#[must_use]
pub fn jpeg_with_tags(gps: bool, tags: &[(&str, &str)]) -> Bytes {
    let mut bytes = solid(4, 4, image::ImageFormat::Jpeg);
    let mut exif = ExifMetadata::new();
    if gps {
        exif.set_tag(gps_latitude());
    }
    for &(name, value) in tags {
        exif.set_tag(text_exif_tag(name, value.to_owned()));
    }
    exif.write_to_vec(&mut bytes, FileExtension::JPEG)
        .expect("write exif fixture");
    Bytes::from(bytes)
}

/// The EXIF `ImageDescription` that `jpeg` carries, if any, for assertions.
#[must_use]
pub fn description(jpeg: &[u8]) -> Option<String> {
    text_tag(jpeg, "ImageDescription")
}

/// The EXIF text tag `name` that `jpeg` carries, if any, for assertions. The
/// names are those [`jpeg_with_tags`] takes.
///
/// # Panics
///
/// Panics on a tag name [`jpeg_with_tags`] does not take.
#[must_use]
pub fn text_tag(jpeg: &[u8], name: &str) -> Option<String> {
    let exif = ExifMetadata::new_from_vec(&jpeg.to_vec(), FileExtension::JPEG).ok()?;
    let value = match exif.get_tag(&text_exif_tag(name, String::new())).next()? {
        ExifTag::Artist(value)
        | ExifTag::Make(value)
        | ExifTag::Model(value)
        | ExifTag::ImageDescription(value) => value,
        _ => return None,
    };
    Some(value.trim_end_matches('\0').to_owned())
}

/// The RGBA pixel at (`x`, `y`) of the image `bytes` decode to, for asserting
/// pixels were redacted. `None` when `bytes` do not decode or the point lies
/// outside the image.
#[must_use]
pub fn pixel(bytes: &[u8], x: u32, y: u32) -> Option<[u8; 4]> {
    let image = image::load_from_memory(bytes).ok()?;
    image.in_bounds(x, y).then(|| image.get_pixel(x, y).0)
}

/// A `width`x`height` solid-colour image encoded as `format`, no metadata.
fn solid(width: u32, height: u32, format: image::ImageFormat) -> Vec<u8> {
    let mut bytes = Vec::new();
    let image = image::RgbImage::from_pixel(width, height, image::Rgb([200, 30, 30]));
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut Cursor::new(&mut bytes), format)
        .expect("encode image fixture");
    bytes
}

/// The GPS latitude tag the GPS fixtures carry.
fn gps_latitude() -> ExifTag {
    ExifTag::GPSLatitude(vec![uR64 {
        nominator: 51,
        denominator: 1,
    }])
}

/// The EXIF text tag `name` holding `value`.
fn text_exif_tag(name: &str, value: String) -> ExifTag {
    match name {
        "Artist" => ExifTag::Artist(value),
        "Make" => ExifTag::Make(value),
        "Model" => ExifTag::Model(value),
        "ImageDescription" => ExifTag::ImageDescription(value),
        other => panic!("no text EXIF tag fixture named `{other}`"),
    }
}

/// A tiny solid-colour TIFF carrying a GPS latitude EXIF tag.
///
/// The TIFF counterpart of [`jpeg_with_gps`]: TIFF is EXIF's native container
/// (the metadata is the file's own IFD structure), so this exercises the
/// EXIF-as-IFD path rather than JPEG's embedded APP1 segment.
#[must_use]
pub fn tiff_with_gps() -> Bytes {
    let mut bytes = solid(4, 4, image::ImageFormat::Tiff);
    // TIFF's EXIF *is* the file's IFD, so a from-scratch `Metadata` lacks the
    // mandatory baseline tags. Read the encoded TIFF's own IFD, add GPS, and
    // write the whole thing back.
    let mut exif =
        ExifMetadata::new_from_vec(&bytes, FileExtension::TIFF).expect("parse tiff fixture ifd");
    exif.set_tag(gps_latitude());
    exif.write_to_vec(&mut bytes, FileExtension::TIFF)
        .expect("write exif fixture");
    Bytes::from(bytes)
}

/// A tiny solid-colour PNG carrying a GPS latitude EXIF tag.
///
/// The PNG counterpart of [`jpeg_with_gps`]: PNG stores EXIF in a separate
/// `eXIf` chunk, so, like JPEG, the metadata layers onto the encoded pixels.
#[must_use]
pub fn png_with_gps() -> Bytes {
    let mut bytes = solid(4, 4, image::ImageFormat::Png);
    let mut exif = ExifMetadata::new();
    exif.set_tag(gps_latitude());
    exif.write_to_vec(
        &mut bytes,
        FileExtension::PNG {
            as_zTXt_chunk: false,
        },
    )
    .expect("write exif fixture");
    Bytes::from(bytes)
}

/// Whether `jpeg` bytes still carry a GPS latitude tag, to assert a strip.
#[must_use]
pub fn has_gps(jpeg: &[u8]) -> bool {
    has_gps_of(jpeg, FileExtension::JPEG)
}

/// Whether `png` bytes still carry a GPS latitude tag, to assert a strip.
#[must_use]
pub fn has_gps_png(png: &[u8]) -> bool {
    has_gps_of(
        png,
        FileExtension::PNG {
            as_zTXt_chunk: false,
        },
    )
}

/// Whether `tiff` bytes still carry a GPS latitude tag, to assert a strip.
#[must_use]
pub fn has_gps_tiff(tiff: &[u8]) -> bool {
    has_gps_of(tiff, FileExtension::TIFF)
}

fn has_gps_of(bytes: &[u8], format: FileExtension) -> bool {
    ExifMetadata::new_from_vec(&bytes.to_vec(), format)
        .map(|meta| {
            meta.get_tag(&ExifTag::GPSLatitude(Vec::new()))
                .next()
                .is_some()
        })
        .unwrap_or(false)
}

/// Whether `bytes` decode as a valid image, to assert an output is intact.
#[must_use]
pub fn is_valid_image(bytes: &[u8]) -> bool {
    image::load_from_memory(bytes).is_ok()
}
