//! Render a surfaced EXIF field's value as the text a reader would see: a
//! string unwrapped, a user comment decoded from its character code, a GPS
//! coordinate in decimal degrees with its hemisphere.

use little_exif::exif_tag::ExifTag;
use little_exif::metadata::Metadata as ExifMetadata;
use little_exif::rational::uR64;

use super::tags::read_tags;

/// The value of the surfaced field `key` in `exif` as text, or `None` when the
/// image carries no readable value for it.
pub(super) fn render(key: &str, exif: &ExifMetadata) -> Option<String> {
    match key {
        "GPSLatitude" => coordinate(
            exif,
            &ExifTag::GPSLatitude(Vec::new()),
            &ExifTag::GPSLatitudeRef(String::new()),
        ),
        "GPSLongitude" => coordinate(
            exif,
            &ExifTag::GPSLongitude(Vec::new()),
            &ExifTag::GPSLongitudeRef(String::new()),
        ),
        _ => read_tags(key)
            .iter()
            .find_map(|tag| exif.get_tag(tag).next().and_then(text)),
    }
}

/// The text of a string or comment tag, trimmed of padding, or `None` when it
/// is empty or not a text tag.
fn text(tag: &ExifTag) -> Option<String> {
    let raw = match tag {
        ExifTag::DateTimeOriginal(s)
        | ExifTag::CreateDate(s)
        | ExifTag::Make(s)
        | ExifTag::Model(s)
        | ExifTag::SerialNumber(s)
        | ExifTag::Artist(s)
        | ExifTag::OwnerName(s)
        | ExifTag::Copyright(s)
        | ExifTag::ImageDescription(s)
        | ExifTag::Software(s)
        | ExifTag::ImageUniqueID(s)
        | ExifTag::GPSLatitudeRef(s)
        | ExifTag::GPSLongitudeRef(s) => s.clone(),
        ExifTag::UserComment(bytes) => user_comment(bytes)?,
        _ => return None,
    };
    let trimmed = raw.trim_matches(|c: char| c == '\0' || c.is_whitespace());
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// A user comment's text: its first eight bytes name the character code
/// (`UNICODE`, `ASCII`, `JIS`, or undefined) and the rest is the comment.
fn user_comment(bytes: &[u8]) -> Option<String> {
    let (code, body) = bytes.split_at_checked(8)?;
    Some(match code {
        b"UNICODE\0" => utf16(body),
        _ => String::from_utf8_lossy(body).into_owned(),
    })
}

/// UTF-16 `body` of unknown byte order (the comment carries none of its own):
/// the order whose high bytes are mostly zero is taken, as text in any Latin
/// script reads that way.
fn utf16(body: &[u8]) -> String {
    let pairs = body.chunks_exact(2);
    let low_first = pairs.clone().filter(|pair| pair[1] == 0).count()
        >= pairs.clone().filter(|pair| pair[0] == 0).count();
    let units = pairs.map(|pair| {
        if low_first {
            u16::from_le_bytes([pair[0], pair[1]])
        } else {
            u16::from_be_bytes([pair[0], pair[1]])
        }
    });
    char::decode_utf16(units)
        .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// A GPS coordinate as decimal degrees with its hemisphere (`51.500700 N`),
/// from its degree, minute, and second rationals and its reference tag.
fn coordinate(exif: &ExifMetadata, tag: &ExifTag, reference: &ExifTag) -> Option<String> {
    let parts = match exif.get_tag(tag).next()? {
        ExifTag::GPSLatitude(parts) | ExifTag::GPSLongitude(parts) => parts,
        _ => return None,
    };
    let degrees: f64 = parts
        .iter()
        .zip([1.0, 60.0, 3600.0])
        .map(|(part, scale)| ratio(part) / scale)
        .sum();
    Some(match exif.get_tag(reference).next().and_then(text) {
        Some(hemisphere) => format!("{degrees:.6} {hemisphere}"),
        None => format!("{degrees:.6}"),
    })
}

/// A rational's value, zero when its denominator is.
fn ratio(part: &uR64) -> f64 {
    if part.denominator == 0 {
        0.0
    } else {
        f64::from(part.nominator) / f64::from(part.denominator)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_comment_is_decoded_from_its_character_code() {
        assert_eq!(
            user_comment(b"ASCII\0\0\0Jane Roe").as_deref(),
            Some("Jane Roe")
        );
        let mut unicode = b"UNICODE\0".to_vec();
        unicode.extend("Jane".encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(user_comment(&unicode).as_deref(), Some("Jane"));
        let mut big_endian = b"UNICODE\0".to_vec();
        big_endian.extend("Jane".encode_utf16().flat_map(u16::to_be_bytes));
        assert_eq!(user_comment(&big_endian).as_deref(), Some("Jane"));
    }

    #[test]
    fn a_string_tag_is_unwrapped_and_trimmed() {
        assert_eq!(
            text(&ExifTag::Artist("Jane Roe\0".to_owned())).as_deref(),
            Some("Jane Roe")
        );
        assert_eq!(text(&ExifTag::Artist("\0".to_owned())), None);
    }
}
