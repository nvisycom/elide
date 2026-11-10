//! The EXIF tag sets: which fields are surfaced and the tags each is read from,
//! which tags a redaction of each logical field must clear, which survive a
//! metadata removal, and which are safe to transfer onto re-encoded pixels.

use little_exif::exif_tag::ExifTag;
use little_exif::ifd::ExifTagGroup;

/// The IFD0 tags that lay out, orient, and colour an image's pixels (TIFF 6.0
/// baseline and extensions, plus the embedded ICC profile). A TIFF cannot be
/// read without them, and none describes the author, the device, or the
/// scene.
const LAYOUT_TAGS: [u16; 39] = [
    0x00FE, 0x00FF, 0x0100, 0x0101, 0x0102, 0x0103, 0x0106, 0x0107, 0x0108, 0x0109, 0x010A, 0x0111,
    0x0115, 0x0116, 0x0117, 0x0118, 0x0119, 0x011A, 0x011B, 0x011C, 0x0128, 0x012D, 0x013D, 0x013E,
    0x013F, 0x0140, 0x0141, 0x0142, 0x0143, 0x0144, 0x0145, 0x0152, 0x0153, 0x0211, 0x0212, 0x0213,
    0x0214, 0x0112, 0x8773,
];

/// The tags that hold EXIF together: the pointers to the EXIF, GPS, and
/// Interop IFDs, and the EXIF and GPS versions.
const STRUCTURE_TAGS: [(u16, ExifTagGroup); 5] = [
    (0x8769, ExifTagGroup::GENERIC),
    (0x8825, ExifTagGroup::GENERIC),
    (0xA005, ExifTagGroup::EXIF),
    (0x9000, ExifTagGroup::EXIF),
    (0x0000, ExifTagGroup::GPS),
];

/// The offset and length of an embedded JPEG: in an image's EXIF the thumbnail
/// (IFD1), a preview of the unredacted pixels; in a TIFF old-style JPEG image
/// data, which is layout.
pub(super) const THUMBNAIL_TAGS: [u16; 2] = [0x0201, 0x0202];

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

/// The keys of the EXIF fields surfaced as metadata, in the order they are
/// streamed. Each is read through [`read_tags`] and cleared through
/// [`tags_for_key`]; the entity label each carries is chosen in `entity`.
///
/// Only fields whose value is text a recognizer can read are surfaced. A
/// vendor's binary maker note cannot be, so it is never surfaced and is
/// stripped under every disposition.
pub(super) const FIELD_KEYS: [&str; 13] = [
    "GPSLatitude",
    "GPSLongitude",
    "DateTimeOriginal",
    "Make",
    "Model",
    "SerialNumber",
    "Artist",
    "OwnerName",
    "Copyright",
    "ImageDescription",
    "UserComment",
    "Software",
    "ImageUniqueID",
];

/// The tags a field `key`'s value is read from: its own tag, and for a
/// coordinate its hemisphere too. Empty for a key this crate does not map.
///
/// Values are placeholders: `get_tag` matches on tag identity, not value.
pub(super) fn read_tags(key: &str) -> Vec<ExifTag> {
    match key {
        // A coordinate reads as its hemisphere too.
        "GPSLatitude" => vec![
            ExifTag::GPSLatitude(Vec::new()),
            ExifTag::GPSLatitudeRef(String::new()),
        ],
        "GPSLongitude" => vec![
            ExifTag::GPSLongitude(Vec::new()),
            ExifTag::GPSLongitudeRef(String::new()),
        ],
        _ => own_tag(key).into_iter().collect(),
    }
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
pub(super) fn tags_for_key(key: &str) -> Vec<ExifTag> {
    match key {
        "GPSLatitude" | "GPSLongitude" => gps_tags(),
        "DateTimeOriginal" => timestamp_tags(),
        "Make" | "Model" | "SerialNumber" => device_tags(),
        _ => own_tag(key).into_iter().collect(),
    }
}

/// The tags an image keeps when its metadata is removed: only what a viewer
/// needs to display it as intended. Orientation decides which way up the pixels
/// are drawn; the colour profile lives outside EXIF and is never touched.
pub(super) fn render_tags() -> [ExifTag; 1] {
    [ExifTag::Orientation(Vec::new())]
}

/// Whether the tag `hex` in an IFD of `group`, in an image of the `tiff`
/// container or another that surfaces the fields `surfaced`, stays when
/// metadata is treated as content: a tag a surfaced field's value is read from,
/// the render tag, a structural pointer, or a TIFF's pixel layout. Everything
/// else in EXIF cannot reach detection, so it is stripped: an embedded
/// thumbnail, a maker note, vendor tags, and the members of a field's group no
/// field reads (a lens serial beside a camera's make, a destination coordinate
/// or altitude beside a position, a create or modify date beside a capture
/// time).
pub(super) fn kept_as_content(
    hex: u16,
    group: ExifTagGroup,
    tiff: bool,
    surfaced: &[&str],
) -> bool {
    let generic = group == ExifTagGroup::GENERIC;
    STRUCTURE_TAGS.contains(&(hex, group))
        || (generic && LAYOUT_TAGS.contains(&hex))
        || (generic && tiff && THUMBNAIL_TAGS.contains(&hex))
        || surfaced.iter().any(|key| {
            read_tags(key)
                .into_iter()
                .any(|field| field.as_u16() == hex && field.get_group() == group)
        })
}

/// The one tag a field `key` names.
fn own_tag(key: &str) -> Option<ExifTag> {
    Some(match key {
        "GPSLatitude" => ExifTag::GPSLatitude(Vec::new()),
        "GPSLongitude" => ExifTag::GPSLongitude(Vec::new()),
        "DateTimeOriginal" => ExifTag::DateTimeOriginal(String::new()),
        "Make" => ExifTag::Make(String::new()),
        "Model" => ExifTag::Model(String::new()),
        "SerialNumber" => ExifTag::SerialNumber(String::new()),
        "Artist" => ExifTag::Artist(String::new()),
        "OwnerName" => ExifTag::OwnerName(String::new()),
        "Copyright" => ExifTag::Copyright(String::new()),
        "ImageDescription" => ExifTag::ImageDescription(String::new()),
        "UserComment" => ExifTag::UserComment(Vec::new()),
        "Software" => ExifTag::Software(String::new()),
        "ImageUniqueID" => ExifTag::ImageUniqueID(String::new()),
        _ => return None,
    })
}

/// The tag that holds `value` as the field `key`'s new value, or `None` when
/// the field cannot hold arbitrary text (a coordinate, a timestamp in its fixed
/// format, a user comment with its character-code prefix) or `value` is blank,
/// which is no field at all. Such a field is removed instead of overwritten.
pub(super) fn written_tag(key: &str, value: &str) -> Option<ExifTag> {
    if value.trim().is_empty() {
        return None;
    }
    let value = value.to_owned();
    Some(match key {
        "Make" => ExifTag::Make(value),
        "Model" => ExifTag::Model(value),
        "SerialNumber" => ExifTag::SerialNumber(value),
        "Artist" => ExifTag::Artist(value),
        "OwnerName" => ExifTag::OwnerName(value),
        "Copyright" => ExifTag::Copyright(value),
        "ImageDescription" => ExifTag::ImageDescription(value),
        "Software" => ExifTag::Software(value),
        "ImageUniqueID" => ExifTag::ImageUniqueID(value),
        _ => return None,
    })
}

/// The whole GPS IFD: the offset pointer plus every coordinate, reference,
/// altitude, time, direction, and destination tag.
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
