//! The EXIF tag sets: which fields are surfaced and the tags each is read from,
//! which tags a redaction of each logical field must clear, which survive a
//! metadata removal, and which are safe to transfer onto re-encoded pixels.

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

/// The keys of the EXIF fields surfaced as metadata, in the order they are
/// streamed. Each is read through [`read_tags`] and cleared through
/// [`tags_for_key`]; the entity label each carries is chosen in `entity`.
pub(super) const FIELD_KEYS: [&str; 14] = [
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
    "MakerNote",
];

/// The tags a field `key`'s value is read from, in priority order: the first
/// the image carries is the value. Empty for a key this crate does not map.
///
/// Values are placeholders: `get_tag` matches on tag identity, not value.
pub(super) fn read_tags(key: &str) -> Vec<ExifTag> {
    match key {
        // A capture time falls back to the create date when the original is
        // absent, so an image carrying only one still surfaces its time.
        "DateTimeOriginal" => vec![
            ExifTag::DateTimeOriginal(String::new()),
            ExifTag::CreateDate(String::new()),
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

/// The one tag a free-standing field `key` names: the authorship, ownership,
/// free-text, and identifier fields, each cleared on its own.
fn own_tag(key: &str) -> Option<ExifTag> {
    Some(match key {
        "GPSLatitude" => ExifTag::GPSLatitude(Vec::new()),
        "GPSLongitude" => ExifTag::GPSLongitude(Vec::new()),
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
        "MakerNote" => ExifTag::MakerNote(Vec::new()),
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
