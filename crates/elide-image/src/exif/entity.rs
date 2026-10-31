//! Surfacing EXIF fields as [`Entity<Metadata>`] values.
//!
//! Reading metadata tells you *what* an image carries; this turns each present
//! privacy-relevant field into an entity so it flows through the same
//! detect/select/redact/audit pipeline as a text span. A stripped GPS tag then
//! shows up in the audit trail beside a redacted name, and a caller can choose,
//! per field, whether to remove it.
//!
//! The key→label table here is the single source of truth: the standalone
//! [`entities`](Source::entities) read and the [`ExifRecognizer`] that runs in
//! the analyze pipeline both classify a field through [`label_for`].

use elide_core::Result;
use elide_core::entity::{Entity, LabelRef, builtins};
use elide_core::modality::metadata::{Metadata, MetadataData};

use super::Source;

/// The recognizer id recorded on each field's detection audit event.
pub(crate) const SOURCE: &str = "exif";

/// The label for an EXIF tag `key`, or `None` when the tag is not one this
/// crate treats as privacy-relevant. The single classification table, shared by
/// the standalone read and the recognizer.
pub(crate) fn label_for(key: &str) -> Option<LabelRef> {
    let label = match key {
        "GPSLatitude" | "GPSLongitude" => &builtins::GEOLOCATION_METADATA,
        "DateTimeOriginal" => &builtins::DATE_TIME,
        "Make" | "Model" | "SerialNumber" | "ImageUniqueID" | "MakerNote" => &builtins::DEVICE_ID,
        "Artist" | "OwnerName" | "Copyright" => &builtins::PERSON_NAME,
        "ImageDescription" | "UserComment" => &builtins::COMMUNICATIONS_CONTENT,
        "Software" => &builtins::PRODUCT,
        _ => return None,
    };
    Some(LabelRef::from(&**label))
}

impl Source<'_> {
    /// One [`Entity<Metadata>`] per privacy-relevant EXIF field the image
    /// actually carries.
    ///
    /// Each entity is addressed by its tag key, labelled by what it exposes
    /// (location, time, device, a person), and carries a deterministic
    /// detection event: an EXIF field is present or it is not, so confidence is
    /// `MAX`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the metadata is present but unparseable.
    pub(crate) fn entities(&self) -> Result<Vec<Entity<Metadata>>> {
        Ok(self
            .values()?
            .into_iter()
            .filter_map(|(key, _)| {
                label_for(key).and_then(|label| Metadata::field_entity(key, label, SOURCE))
            })
            .collect())
    }

    /// Each surfaced EXIF field the image carries, as a [`MetadataData`], for
    /// a caller (the codec) that streams fields to a recognizer. Every surfaced
    /// field has a label, so the recognizer classifies each one.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the metadata is present but unparseable.
    pub(crate) fn fields(&self) -> Result<Vec<MetadataData>> {
        Ok(self
            .values()?
            .into_iter()
            .map(|(key, value)| MetadataData::new(key.to_owned(), value))
            .collect())
    }
}

#[cfg(all(test, feature = "jpeg"))]
mod tests {
    use little_exif::exif_tag::ExifTag;
    use little_exif::filetype::FileExtension;
    use little_exif::metadata::Metadata as ExifMetadata;

    use super::*;
    use crate::exif::tags::FIELD_KEYS;

    /// A JPEG carrying the given EXIF tags.
    fn jpeg_with(tags: Vec<ExifTag>) -> Vec<u8> {
        let mut bytes = Vec::new();
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([1, 2, 3]));
        image::DynamicImage::ImageRgb8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Jpeg,
            )
            .expect("encode jpeg");
        let mut exif = ExifMetadata::new();
        for tag in tags {
            exif.set_tag(tag);
        }
        exif.write_to_vec(&mut bytes, FileExtension::JPEG)
            .expect("write exif");
        bytes
    }

    fn source(bytes: &[u8]) -> Source<'_> {
        Source::new(bytes, FileExtension::JPEG)
    }

    #[test]
    fn one_entity_per_present_field_labelled_by_kind() {
        let bytes = jpeg_with(vec![
            ExifTag::Make("Nvisy".into()),
            ExifTag::DateTimeOriginal("2024:01:02 03:04:05".into()),
            ExifTag::GPSLatitude(vec![little_exif::rational::uR64 {
                nominator: 51,
                denominator: 1,
            }]),
        ]);
        let entities = source(&bytes).entities().expect("entities");

        // GPS + timestamp + device make — three fields, three entities.
        assert_eq!(entities.len(), 3);

        let at = |key: &str| entities.iter().find(|e| e.location.key == key);
        let gps = at("GPSLatitude").expect("gps entity");
        assert_eq!(gps.label, LabelRef::from(&*builtins::GEOLOCATION_METADATA));
        let time = at("DateTimeOriginal").expect("time entity");
        assert_eq!(time.label, LabelRef::from(&*builtins::DATE_TIME));
        let make = at("Make").expect("make entity");
        assert_eq!(make.label, LabelRef::from(&*builtins::DEVICE_ID));

        // Each carries a detection event that verifies against its audit chain.
        assert!(gps.audit.verify().is_ok());
    }

    #[test]
    fn no_entities_without_exif() {
        let bytes = jpeg_with(Vec::new());
        assert!(source(&bytes).entities().expect("entities").is_empty());
    }

    #[test]
    fn every_surfaced_field_has_a_label() {
        for key in FIELD_KEYS {
            assert!(label_for(key).is_some(), "{key} has no label");
        }
    }
}
