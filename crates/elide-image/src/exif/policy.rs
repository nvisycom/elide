//! [`ExifPolicy`]: what to do with an image's EXIF metadata on re-encode.

/// What to do with an image's EXIF metadata when re-encoding it.
///
/// EXIF mixes privacy-sensitive fields (GPS, device serial, capture timestamp)
/// with benign ones (orientation, colour profile) that a viewer needs to render
/// the image correctly, so the choice is a policy, not a fixed behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
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
