//! The EXIF tag sets: which `little_exif` tags a redaction of each logical
//! field must clear, which are safe to transfer, and which the sensitive-strip
//! policy removes.

use little_exif::exif_tag::ExifTag;

/// Whether `tag` is a benign IFD0 (GENERIC) descriptive tag safe to carry from
/// the source onto a freshly-encoded TIFF.
///
/// An allowlist, deliberately: IFD0 holds both descriptive metadata and the
/// pixel-layout tags (strips, tiles, planar config, dimensions, compression,
/// resolution, colour), and `little_exif` surfaces any tag it does not model as
/// an `Unknown*` variant — so a denylist could never be exhaustive, and copying
/// a stale layout tag would point the output IFD at the wrong pixels. Only the
/// tags named here cross; everything else in IFD0 stays as the encoder wrote it.
/// The sub-IFDs (EXIF/GPS/Interop) are pure metadata and transfer wholesale, so
/// their tags need not be listed.
#[cfg(feature = "exif")]
pub(super) fn is_transferable_ifd0_tag(tag: &ExifTag) -> bool {
    matches!(
        tag,
        ExifTag::Make(_)
            | ExifTag::Model(_)
            | ExifTag::Software(_)
            | ExifTag::Artist(_)
            | ExifTag::Copyright(_)
            | ExifTag::ImageDescription(_)
            | ExifTag::ModifyDate(_)
            | ExifTag::Orientation(_)
            | ExifTag::ExifOffset(_)
            | ExifTag::GPSInfo(_)
    )
}

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
pub(super) fn tags_for_key(key: &str) -> Vec<ExifTag> {
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
/// [`StripSensitive`](crate::exif::ExifPolicy::StripSensitive): every field that
/// can place a photo (GPS), time it (capture/create/modify/offset), or tie it to
/// a person or device (make/model/serials, owner, artist, copyright, software,
/// unique id, and the free-text comment/description fields that routinely carry
/// names or notes).
///
/// Render-critical tags (orientation, resolution, colour) are deliberately left
/// in, so the stripped image still displays as intended. Values here are
/// placeholders: `remove_tag` matches on tag identity, not on the value.
///
/// This is a denylist, not an allowlist: a proprietary MakerNote sub-tag not
/// modelled by `little_exif`, or a new tag added to a future EXIF revision, can
/// slip through. For a hard guarantee that nothing personal survives, use
/// [`Strip`](crate::exif::ExifPolicy::Strip), which drops the block wholesale.
pub(super) fn sensitive_tags() -> Vec<ExifTag> {
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
