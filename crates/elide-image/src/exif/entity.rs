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
use elide_core::modality::metadata::{Metadata, MetadataData, field_entity};

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
        "Make" | "Model" | "SerialNumber" => &builtins::DEVICE_ID,
        _ => return None,
    };
    Some(LabelRef::from(&**label))
}

impl Source<'_> {
    /// One [`Entity<Metadata>`] per privacy-relevant EXIF field the image
    /// actually carries.
    ///
    /// Each entity is addressed by its tag key, labelled by what it exposes
    /// (location, time, device), and carries a deterministic detection event: an
    /// EXIF field is present or it is not, so confidence is `MAX`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the metadata is present but unparseable.
    pub(crate) fn entities(&self) -> Result<Vec<Entity<Metadata>>> {
        let meta = self.read()?;
        let mut entities = Vec::new();
        let mut push = |present: bool, key: &'static str| {
            if present
                && let Some(label) = label_for(key)
                && let Some(entity) = field_entity(key, label, SOURCE)
            {
                entities.push(entity);
            }
        };
        push(meta.gps_latitude.is_some(), "GPSLatitude");
        push(meta.gps_longitude.is_some(), "GPSLongitude");
        push(meta.timestamp.is_some(), "DateTimeOriginal");
        push(meta.device_make.is_some(), "Make");
        push(meta.device_model.is_some(), "Model");
        push(meta.device_serial.is_some(), "SerialNumber");
        Ok(entities)
    }

    /// Each privacy-relevant EXIF field the image carries, as a
    /// [`MetadataData`], for a caller (the codec) that streams fields to a
    /// recognizer.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the metadata is present but unparseable.
    pub(crate) fn fields(&self) -> Result<Vec<MetadataData>> {
        let meta = self.read()?;
        let mut fields = Vec::new();
        let mut push = |key: &str, value: Option<String>| {
            if let Some(value) = value
                && label_for(key).is_some()
            {
                fields.push(MetadataData::new(key.to_owned(), value));
            }
        };
        push("GPSLatitude", meta.gps_latitude);
        push("GPSLongitude", meta.gps_longitude);
        push("DateTimeOriginal", meta.timestamp);
        push("Make", meta.device_make);
        push("Model", meta.device_model);
        push("SerialNumber", meta.device_serial);
        Ok(fields)
    }
}

#[cfg(all(test, feature = "jpeg"))]
mod tests {
    use little_exif::exif_tag::ExifTag;
    use little_exif::filetype::FileExtension;
    use little_exif::metadata::Metadata as ExifMetadata;

    use super::*;

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
}
