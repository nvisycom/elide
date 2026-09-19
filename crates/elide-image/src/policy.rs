//! [`ExifPolicy`]: what to do with an image's EXIF metadata on re-encode.
//!
//! A dependency-free configuration value, so it is always available — a caller
//! can name a policy without enabling the `exif` feature and its metadata
//! engine (`little_exif`). [`ImageBuffer::encode`](crate::ImageBuffer::encode)
//! consumes one; applying it needs the `exif` feature, but expressing the
//! intent does not.

/// What to do with an image's EXIF metadata when re-encoding it.
///
/// EXIF mixes privacy-sensitive fields (GPS, device serial, capture timestamp)
/// with benign ones (orientation, colour profile) that a viewer needs to render
/// the image correctly, so the choice is a policy, not a fixed behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum ExifPolicy {
    /// Drop the entire metadata block. The safest default: nothing personal
    /// survives, though benign fields (a viewer's orientation hint) go too.
    #[default]
    StripAll,
    /// Drop only the privacy-sensitive fields (GPS, device, timestamps), keeping
    /// the rest so the image still renders as intended.
    StripSensitive,
    /// Leave the metadata untouched.
    Keep,
}

#[cfg(all(test, feature = "schema"))]
mod tests {
    use super::ExifPolicy;

    /// The `schema` feature derives `JsonSchema`, so a caller can emit a JSON
    /// schema for a config surface `ExifPolicy` appears in.
    #[test]
    fn derives_json_schema() {
        let schema = schemars::schema_for!(ExifPolicy);
        let json = serde_json::to_string(&schema).expect("schema serializes");
        // The three variants surface in the generated schema.
        assert!(json.contains("strip_all"));
        assert!(json.contains("strip_sensitive"));
        assert!(json.contains("keep"));
    }
}
