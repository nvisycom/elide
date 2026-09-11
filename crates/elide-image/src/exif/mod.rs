//! EXIF metadata read, strip, and field-level removal.
//!
//! Backed by `little_exif`, the one permissive pure-Rust crate that both reads
//! and edits EXIF for JPEG, PNG, and TIFF. Its parser can panic on malformed
//! input, so
//! every call into it is wrapped in a panic guard ([`guard`]): a panic becomes a
//! fail-closed error, never an aborted process, since this runs on the
//! redaction path where a crash mid-strip is unacceptable.
//!
//! The entry point is [`ImageBuffer`](crate::ImageBuffer), which opens the image
//! once and drives these helpers with a format it already knows; nothing here
//! sniffs magic bytes.

mod entity;
mod policy;
mod recognizer;

use std::panic::{AssertUnwindSafe, catch_unwind};

use elide_core::{Error, ErrorKind, Result};
use little_exif::exif_tag::ExifTag;
use little_exif::filetype::FileExtension;
use little_exif::metadata::Metadata as ExifMetadata;

pub use self::policy::ExifPolicy;
pub use self::recognizer::ExifRecognizer;

/// The privacy-relevant metadata read out of an image: the fields that can
/// identify a person, place, device, or time.
///
/// Not every EXIF field, the ones that carry personal information: where a photo
/// was taken (GPS), when (timestamps), and on what device (make/model/serial).
/// Each is `None` when the image does not carry it.
///
/// Internal: the raw values feed [`Source::entities`], the public surface for
/// what metadata an image carries. The values themselves are not exposed, so
/// this exists only where that reader does.
#[cfg(feature = "exif")]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) struct Metadata {
    /// GPS latitude, as the image records it.
    pub(crate) gps_latitude: Option<String>,
    /// GPS longitude.
    pub(crate) gps_longitude: Option<String>,
    /// When the photo was taken (`DateTimeOriginal`), else the create date.
    pub(crate) timestamp: Option<String>,
    /// Camera / device manufacturer (`Make`).
    pub(crate) device_make: Option<String>,
    /// Camera / device model (`Model`).
    pub(crate) device_model: Option<String>,
    /// Device / camera serial number.
    pub(crate) device_serial: Option<String>,
}

/// A borrowed image container with its format, the receiver for the metadata
/// operations.
///
/// A thin lifetime wrapper over `&[u8]` so the bytes and the format that
/// interprets them travel together, and the read/strip/transfer operations read
/// as methods on the source rather than free functions taking a loose pair.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Source<'a> {
    bytes: &'a [u8],
    format: FileExtension,
}

impl<'a> Source<'a> {
    /// Wrap `bytes` interpreted as `format`.
    pub(crate) fn new(bytes: &'a [u8], format: FileExtension) -> Self {
        Self { bytes, format }
    }

    /// Parse the source's EXIF, or `None` when it carries none.
    fn parse(&self) -> Result<Option<ExifMetadata>> {
        let buffer = self.bytes.to_vec();
        let format = self.format;
        Ok(guard(|| ExifMetadata::new_from_vec(&buffer, format))?.ok())
    }

    /// Read the privacy-relevant metadata. Empty when the image carries no EXIF
    /// (the common case for PNG).
    #[cfg(feature = "exif")]
    pub(crate) fn read(&self) -> Result<Metadata> {
        let Some(exif) = self.parse()? else {
            return Ok(Metadata::default());
        };
        let text = |tag: &ExifTag| {
            guard(|| exif.get_tag(tag).next().map(|t| format!("{t:?}")))
                .ok()
                .flatten()
        };
        Ok(Metadata {
            gps_latitude: text(&ExifTag::GPSLatitude(Vec::new())),
            gps_longitude: text(&ExifTag::GPSLongitude(Vec::new())),
            timestamp: text(&ExifTag::DateTimeOriginal(String::new()))
                .or_else(|| text(&ExifTag::CreateDate(String::new()))),
            device_make: text(&ExifTag::Make(String::new())),
            device_model: text(&ExifTag::Model(String::new())),
            device_serial: text(&ExifTag::SerialNumber(String::new())),
        })
    }

    /// Apply `policy`, returning the resulting container bytes.
    pub(crate) fn strip(&self, policy: ExifPolicy) -> Result<Vec<u8>> {
        match policy {
            ExifPolicy::Keep => Ok(self.bytes.to_vec()),
            ExifPolicy::StripAll => self.strip_all(),
            ExifPolicy::StripSensitive => self.strip_sensitive(),
        }
    }

    /// Clear every metadata field, returning the result.
    ///
    /// Clears the EXIF block wholesale, then runs the baseline scrub so the
    /// APP12/APP13 provenance segments go too: `clear_metadata` addresses the
    /// EXIF block but not those dedicated application segments.
    fn strip_all(&self) -> Result<Vec<u8>> {
        let mut buf = self.bytes.to_vec();
        let format = self.format;
        guard(|| ExifMetadata::clear_metadata(&mut buf, format))?
            .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("exif strip: {e}")))?;
        self.baseline_scrub(&mut buf)?;
        Ok(buf)
    }

    /// Remove only the privacy fields, leaving the rest intact.
    ///
    /// When the EXIF block is absent or cannot be decoded, `little_exif` reads no
    /// tags, so a selective removal has nothing to act on and would leave
    /// undecodable EXIF in place. That case fails closed to
    /// [`strip_all`](Self::strip_all), which clears the block by bytes; a genuine
    /// no-EXIF image is a harmless no-op there. Either way the baseline scrub of
    /// the APP12/APP13 provenance segments always runs.
    fn strip_sensitive(&self) -> Result<Vec<u8>> {
        let Some(mut exif) = self.parse()? else {
            // No decodable EXIF: clear the block by bytes rather than trusting a
            // possibly-undecodable block was truly absent, and scrub the rest.
            return self.strip_all();
        };
        guard(AssertUnwindSafe(|| {
            for tag in sensitive_tags() {
                exif.remove_tag(tag);
            }
        }))?;
        let mut buf = self.write(&exif)?;
        self.baseline_scrub(&mut buf)?;
        Ok(buf)
    }

    /// Remove exactly the fields named by `keys` (each an EXIF tag key such as
    /// `"GPSLatitude"`), leaving all others intact, then run the format's
    /// mandatory baseline scrub, returning the resulting container bytes.
    ///
    /// Unlike [`strip_sensitive`](Self::strip_sensitive), which drops the whole
    /// privacy set, this removes only the caller's chosen keys, so a per-field
    /// redaction decision (from picked entities) maps straight to the byte edit.
    /// A key not present, or not one this crate maps to a tag, is skipped. The
    /// baseline scrub always runs, even when `keys` is empty.
    #[cfg(feature = "exif")]
    pub(crate) fn remove_keys(&self, keys: &[&str]) -> Result<Vec<u8>> {
        let tags: Vec<ExifTag> = keys.iter().flat_map(|k| tags_for_key(k)).collect();
        let mut buf = match self.parse()? {
            Some(mut exif) if !tags.is_empty() => {
                guard(AssertUnwindSafe(|| {
                    for tag in tags {
                        exif.remove_tag(tag);
                    }
                }))?;
                self.write(&exif)?
            }
            // No EXIF, or nothing to remove: the source bytes, still subject to
            // the baseline scrub below.
            _ => self.bytes.to_vec(),
        };
        self.baseline_scrub(&mut buf)?;
        Ok(buf)
    }

    /// The format's mandatory baseline scrub, applied on every metadata edit
    /// regardless of which fields were picked.
    ///
    /// For JPEG this clears the APP12 (Ducky) and APP13 (Photoshop/IPTC)
    /// application segments: whole blocks that carry provenance (IPTC captions,
    /// credit, Photoshop metadata) and are not individual EXIF tags, so they
    /// cannot be addressed as fields. Other formats have no such segments.
    #[cfg(feature = "exif")]
    fn baseline_scrub(&self, buf: &mut Vec<u8>) -> Result<()> {
        if !matches!(self.format, FileExtension::JPEG) {
            return Ok(());
        }
        let format = self.format;
        guard(|| ExifMetadata::clear_app12_segment(buf, format))?
            .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("app12 scrub: {e}")))?;
        guard(|| ExifMetadata::clear_app13_segment(buf, format))?
            .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("app13 scrub: {e}")))?;
        Ok(())
    }

    /// Copy this source's full metadata onto `dest` (freshly re-encoded pixels
    /// that carry none), so a [`Keep`](ExifPolicy::Keep) policy survives a pixel
    /// edit.
    pub(crate) fn transfer(&self, dest: Vec<u8>) -> Result<Vec<u8>> {
        let Some(exif) = self.parse()? else {
            // Source had no metadata: nothing to carry over.
            return Ok(dest);
        };
        // TIFF stores EXIF *as* the file's IFD, so `little_exif`'s TIFF writer
        // rebuilds the whole file from the metadata and ignores `dest`'s pixels
        // — the JPEG/PNG segment-layering below would silently drop the pixel
        // redactions. Instead transplant this source's non-structural tags onto
        // the freshly-encoded (redacted) container's own metadata, which already
        // carries the redacted pixels as its strip data.
        if matches!(self.format, FileExtension::TIFF) {
            return self.transfer_tiff(&exif, dest);
        }
        let format = self.format;
        let mut out = dest;
        guard(AssertUnwindSafe(|| exif.write_to_vec(&mut out, format)))?
            .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("exif transfer: {e}")))?;
        Ok(out)
    }

    /// The TIFF path for [`transfer`](Self::transfer): parse the redacted
    /// container `dest` (whose metadata already holds the redacted pixels and the
    /// mandatory TIFF-structural tags) and copy every non-structural tag from
    /// `source` onto it, so the output carries the redacted pixels plus the
    /// source's kept EXIF.
    fn transfer_tiff(&self, source: &ExifMetadata, dest: Vec<u8>) -> Result<Vec<u8>> {
        let mut out = guard(|| ExifMetadata::new_from_vec(&dest, FileExtension::TIFF))?
            .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("tiff reparse: {e}")))?;
        guard(AssertUnwindSafe(|| {
            for tag in source {
                if !is_tiff_structural(tag) {
                    out.set_tag(tag.clone());
                }
            }
        }))?;
        let mut buf = dest;
        guard(AssertUnwindSafe(|| {
            out.write_to_vec(&mut buf, FileExtension::TIFF)
        }))?
        .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("tiff transfer: {e}")))?;
        Ok(buf)
    }

    /// Write `exif` back onto this source's bytes.
    fn write(&self, exif: &ExifMetadata) -> Result<Vec<u8>> {
        let format = self.format;
        let mut buf = self.bytes.to_vec();
        guard(AssertUnwindSafe(|| exif.write_to_vec(&mut buf, format)))?
            .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("exif rewrite: {e}")))?;
        Ok(buf)
    }
}

/// Whether `tag` is a TIFF-structural tag that describes the pixel layout
/// (dimensions, strips, compression, resolution) rather than carried metadata.
///
/// On the TIFF transfer path these must stay as the freshly-encoded container's
/// own values — copying the source's would point the IFD at the original,
/// un-redacted pixels. Mirrors the set `little_exif`'s `reduce_to_a_minimum`
/// preserves.
#[cfg(feature = "exif")]
fn is_tiff_structural(tag: &ExifTag) -> bool {
    matches!(
        tag,
        ExifTag::StripOffsets(_, _)
            | ExifTag::StripByteCounts(_)
            | ExifTag::ThumbnailOffset(_, _)
            | ExifTag::ThumbnailLength(_)
            | ExifTag::ImageWidth(_)
            | ExifTag::ImageHeight(_)
            | ExifTag::BitsPerSample(_)
            | ExifTag::Compression(_)
            | ExifTag::PhotometricInterpretation(_)
            | ExifTag::SamplesPerPixel(_)
            | ExifTag::RowsPerStrip(_)
            | ExifTag::XResolution(_)
            | ExifTag::YResolution(_)
            | ExifTag::ResolutionUnit(_)
            | ExifTag::ColorMap(_)
    )
}

/// The `little_exif` tag matching a field `key` this crate surfaces as an
/// Every tag a picked field `key` must remove, or empty for a key this crate
/// does not map.
///
/// The write-side inverse of the key→label table, and deliberately *broader*
/// than the single tag a key names: a field is surfaced under one representative
/// key (e.g. `GPSLatitude`), but the entity it produces is labelled by what it
/// *exposes* (a location), so redacting it must clear the whole logical group,
/// the entire GPS IFD for a coordinate, all timestamps for a capture time, and
/// device + lens identity for a device tag. Removing only the one named tag
/// would leave the hemisphere refs, altitude, GPS timestamp, and destination
/// coordinates behind, so a "geolocation" redaction would still ship location.
///
/// Values are placeholders: `remove_tag` matches on tag identity, not value.
#[cfg(feature = "exif")]
fn tags_for_key(key: &str) -> Vec<ExifTag> {
    match key {
        "GPSLatitude" | "GPSLongitude" => gps_tags(),
        "DateTimeOriginal" => timestamp_tags(),
        "Make" | "Model" | "SerialNumber" => device_tags(),
        _ => Vec::new(),
    }
}

/// The whole GPS IFD: the offset pointer plus every coordinate, reference,
/// altitude, time, direction, and destination tag.
#[cfg(feature = "exif")]
fn gps_tags() -> Vec<ExifTag> {
    vec![
        ExifTag::GPSInfo(Vec::new()),
        ExifTag::GPSLatitudeRef(String::new()),
        ExifTag::GPSLatitude(Vec::new()),
        ExifTag::GPSLongitudeRef(String::new()),
        ExifTag::GPSLongitude(Vec::new()),
        ExifTag::GPSAltitudeRef(Vec::new()),
        ExifTag::GPSAltitude(Vec::new()),
        ExifTag::GPSTimeStamp(Vec::new()),
        ExifTag::GPSDateStamp(String::new()),
        ExifTag::GPSImgDirectionRef(String::new()),
        ExifTag::GPSImgDirection(Vec::new()),
        ExifTag::GPSDestLatitudeRef(String::new()),
        ExifTag::GPSDestLatitude(Vec::new()),
        ExifTag::GPSDestLongitudeRef(String::new()),
        ExifTag::GPSDestLongitude(Vec::new()),
    ]
}

/// Every capture/create/modify timestamp and its UTC offset.
#[cfg(feature = "exif")]
fn timestamp_tags() -> Vec<ExifTag> {
    vec![
        ExifTag::DateTimeOriginal(String::new()),
        ExifTag::CreateDate(String::new()),
        ExifTag::ModifyDate(String::new()),
        ExifTag::OffsetTime(String::new()),
        ExifTag::OffsetTimeOriginal(String::new()),
        ExifTag::OffsetTimeDigitized(String::new()),
    ]
}

/// Device and lens identity, including the serials that fingerprint a body.
#[cfg(feature = "exif")]
fn device_tags() -> Vec<ExifTag> {
    vec![
        ExifTag::Make(String::new()),
        ExifTag::Model(String::new()),
        ExifTag::SerialNumber(String::new()),
        ExifTag::LensMake(String::new()),
        ExifTag::LensModel(String::new()),
        ExifTag::LensSerialNumber(String::new()),
    ]
}

/// The privacy-sensitive tags removed by
/// [`StripSensitive`](ExifPolicy::StripSensitive): every field that can place a
/// photo (GPS), time it (capture/create/modify/offset), or tie it to a person or
/// device (make/model/serials, owner, artist, copyright, software, unique id, and
/// the free-text comment/description fields that routinely carry names or notes).
///
/// Render-critical tags (orientation, resolution, colour) are deliberately left
/// in, so the stripped image still displays as intended. Values here are
/// placeholders: `remove_tag` matches on tag identity, not on the value.
///
/// This is a denylist, not an allowlist: a proprietary MakerNote sub-tag not
/// modelled by `little_exif`, or a new tag added to a future EXIF revision, can
/// slip through. For a hard guarantee that nothing personal survives, use
/// [`StripAll`](ExifPolicy::StripAll), which drops the block wholesale.
fn sensitive_tags() -> Vec<ExifTag> {
    let mut tags = gps_tags();
    tags.extend(timestamp_tags());
    tags.extend(device_tags());
    // Authorship, ownership, and free-text fields that carry names/notes.
    tags.extend([
        ExifTag::OwnerName(String::new()),
        ExifTag::Artist(String::new()),
        ExifTag::Copyright(String::new()),
        ExifTag::Software(String::new()),
        ExifTag::ImageDescription(String::new()),
        ExifTag::ImageUniqueID(String::new()),
        ExifTag::UserComment(Vec::new()),
        ExifTag::MakerNote(Vec::new()),
    ]);
    tags
}

/// Run `f`, turning a panic (little_exif can panic on malformed input) into a
/// fail-closed [`ErrorKind::MalformedInput`] instead of aborting.
fn guard<T>(f: impl FnOnce() -> T) -> Result<T> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|_| {
        Error::new(
            ErrorKind::MalformedInput,
            "image metadata parser panicked on malformed input",
        )
    })
}

// These tests read back the structured metadata to verify a strip, so they need
// the same `read`/`Metadata` the `entities` feature compiles in.
#[cfg(all(test, feature = "jpeg", feature = "exif"))]
mod tests {
    use image::{ImageFormat as ImgFormat, RgbImage};

    use super::*;

    /// Whether the read carries no privacy-relevant field.
    fn is_empty(meta: &Metadata) -> bool {
        meta.gps_latitude.is_none()
            && meta.gps_longitude.is_none()
            && meta.timestamp.is_none()
            && meta.device_make.is_none()
            && meta.device_model.is_none()
            && meta.device_serial.is_none()
    }

    /// A minimal JPEG carrying the given EXIF tags, as raw container bytes.
    fn jpeg_with(tags: Vec<ExifTag>) -> Vec<u8> {
        // A 2x2 solid image, encoded to JPEG: real container the parser accepts.
        let mut bytes = Vec::new();
        let img = RgbImage::from_pixel(2, 2, image::Rgb([200, 30, 30]));
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImgFormat::Jpeg)
            .expect("encode jpeg");

        let mut exif = ExifMetadata::new();
        for tag in tags {
            exif.set_tag(tag);
        }
        exif.write_to_vec(&mut bytes, FileExtension::JPEG)
            .expect("write exif");
        bytes
    }

    /// A JPEG with no EXIF at all.
    fn jpeg_bare() -> Vec<u8> {
        jpeg_with(Vec::new())
    }

    fn source(bytes: &[u8]) -> Source<'_> {
        Source::new(bytes, FileExtension::JPEG)
    }

    #[test]
    fn read_surfaces_the_privacy_fields() {
        let bytes = jpeg_with(vec![
            ExifTag::Make("Nvisy".into()),
            ExifTag::Model("X100".into()),
            ExifTag::SerialNumber("SN-42".into()),
            ExifTag::DateTimeOriginal("2024:01:02 03:04:05".into()),
        ]);
        let meta = source(&bytes).read().expect("read");
        assert!(meta.device_make.unwrap().contains("Nvisy"));
        assert!(meta.device_model.unwrap().contains("X100"));
        assert!(meta.device_serial.unwrap().contains("SN-42"));
        assert!(meta.timestamp.unwrap().contains("2024"));
    }

    #[test]
    fn read_is_empty_without_exif() {
        let bytes = jpeg_bare();
        let meta = source(&bytes).read().expect("read");
        assert!(is_empty(&meta));
    }

    #[test]
    fn strip_all_clears_every_field() {
        let bytes = jpeg_with(vec![
            ExifTag::Make("Nvisy".into()),
            ExifTag::GPSLatitude(vec![little_exif::rational::uR64 {
                nominator: 51,
                denominator: 1,
            }]),
        ]);
        let stripped = source(&bytes)
            .strip(ExifPolicy::StripAll)
            .expect("strip all");
        let meta = source(&stripped).read().expect("read back");
        assert!(is_empty(&meta), "strip_all left: {meta:?}");
    }

    #[test]
    fn strip_sensitive_removes_gps_time_and_device() {
        let bytes = jpeg_with(vec![
            ExifTag::Make("Nvisy".into()),
            ExifTag::Model("X100".into()),
            ExifTag::SerialNumber("SN-42".into()),
            ExifTag::DateTimeOriginal("2024:01:02 03:04:05".into()),
            ExifTag::GPSLatitude(vec![little_exif::rational::uR64 {
                nominator: 51,
                denominator: 1,
            }]),
        ]);
        let stripped = source(&bytes)
            .strip(ExifPolicy::StripSensitive)
            .expect("strip sensitive");
        let meta = source(&stripped).read().expect("read back");
        assert!(is_empty(&meta), "sensitive fields survived: {meta:?}");
    }

    #[test]
    fn keep_leaves_the_bytes_untouched() {
        let bytes = jpeg_with(vec![ExifTag::Make("Nvisy".into())]);
        let kept = source(&bytes).strip(ExifPolicy::Keep).expect("keep");
        assert_eq!(kept, bytes);
    }

    #[test]
    fn strip_sensitive_on_a_bare_image_is_a_successful_no_op() {
        // No decodable EXIF: strip_sensitive falls through to the byte-level
        // clear, which must succeed (not error) on an image that simply has no
        // metadata, and leave it readable with nothing sensitive.
        let bytes = jpeg_bare();
        let stripped = source(&bytes)
            .strip(ExifPolicy::StripSensitive)
            .expect("strip sensitive on bare image must succeed");
        let meta = source(&stripped).read().expect("read back");
        assert!(is_empty(&meta));
    }

    /// The same bare-image fallthrough on PNG, whose format has no APP12/APP13
    /// segments (so `baseline_scrub` is a no-op): the byte-level clear must still
    /// succeed rather than error on an image that carries no metadata.
    #[cfg(feature = "png")]
    #[test]
    fn strip_sensitive_on_a_bare_png_is_a_successful_no_op() {
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(RgbImage::from_pixel(2, 2, image::Rgb([10, 20, 30])))
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImgFormat::Png)
            .expect("encode png");
        let src = Source::new(
            &bytes,
            FileExtension::PNG {
                as_zTXt_chunk: false,
            },
        );
        let stripped = src
            .strip(ExifPolicy::StripSensitive)
            .expect("strip sensitive on bare png must succeed");
        assert!(
            Source::new(
                &stripped,
                FileExtension::PNG {
                    as_zTXt_chunk: false
                }
            )
            .read()
            .map(|m| is_empty(&m))
            .unwrap_or(true)
        );
    }

    #[test]
    fn transfer_carries_metadata_onto_fresh_bytes() {
        let with_exif = jpeg_with(vec![ExifTag::Make("Nvisy".into())]);
        let fresh = jpeg_bare();
        let merged = source(&with_exif).transfer(fresh).expect("transfer");
        let meta = source(&merged).read().expect("read back");
        assert!(meta.device_make.unwrap().contains("Nvisy"));
    }

    /// Picking the surfaced `GPSLatitude` field removes the WHOLE GPS IFD, not
    /// just the latitude value: the hemisphere refs, altitude, GPS timestamp,
    /// and direction must all be gone, or a "geolocation" redaction still ships
    /// location-adjacent data.
    #[test]
    fn removing_a_gps_key_clears_the_whole_gps_ifd() {
        let ur = |n| {
            vec![little_exif::rational::uR64 {
                nominator: n,
                denominator: 1,
            }]
        };
        let bytes = jpeg_with(vec![
            ExifTag::GPSLatitude(ur(51)),
            ExifTag::GPSLatitudeRef("N".into()),
            ExifTag::GPSLongitude(ur(0)),
            ExifTag::GPSLongitudeRef("W".into()),
            ExifTag::GPSAltitude(ur(100)),
            ExifTag::GPSTimeStamp(ur(12)),
            ExifTag::GPSDateStamp("2024:01:02".into()),
        ]);
        // The picked key is the one the entity surfaces.
        let stripped = source(&bytes).remove_keys(&["GPSLatitude"]).expect("strip");

        // Every GPS tag must be gone, read straight from little_exif so no tag
        // the structured `Metadata` view omits can hide.
        let exif = ExifMetadata::new_from_vec(&stripped, FileExtension::JPEG).expect("parse");
        let present = |tag: &ExifTag| exif.get_tag(tag).next().is_some();
        assert!(
            !present(&ExifTag::GPSLatitude(Vec::new())),
            "latitude survived"
        );
        assert!(
            !present(&ExifTag::GPSLatitudeRef(String::new())),
            "lat ref survived"
        );
        assert!(
            !present(&ExifTag::GPSLongitude(Vec::new())),
            "longitude survived"
        );
        assert!(
            !present(&ExifTag::GPSLongitudeRef(String::new())),
            "lon ref survived"
        );
        assert!(
            !present(&ExifTag::GPSAltitude(Vec::new())),
            "altitude survived"
        );
        assert!(
            !present(&ExifTag::GPSTimeStamp(Vec::new())),
            "gps time survived"
        );
        assert!(
            !present(&ExifTag::GPSDateStamp(String::new())),
            "gps date survived"
        );
    }
}
