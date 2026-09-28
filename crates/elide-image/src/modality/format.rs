//! [`ImageFormat`]: the raster formats this crate names.

/// A raster image format.
///
/// A closed set, not a re-export of [`image::ImageFormat`]: the `image` crate
/// knows many formats this crate does not model, so a caller can only name one
/// of these, and an unsupported input format is rejected at the boundary rather
/// than failing deeper in.
///
/// The variants are always present — naming a format is data, independent of
/// whether this build can decode it. Whether a codec is compiled in is a
/// separate, runtime question: [`can_decode`](Self::can_decode) answers it, and
/// [`ImageBuffer::open`](crate::ImageBuffer::open) returns a capability error
/// for a format whose feature is off. This lets a caller that only needs to
/// *name* a format (a remote OCR backend that never decodes locally) do so in
/// any build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum ImageFormat {
    /// PNG.
    Png,
    /// JPEG.
    Jpeg,
    /// TIFF.
    Tiff,
}

impl ImageFormat {
    /// The format a filename `extension` names (case-insensitive), or `None`
    /// when it is not one this crate models.
    ///
    /// A filename-based *hint* only, and independent of build features: the
    /// authoritative format is the one the caller resolved at ingestion, and
    /// whether this build can decode it is [`can_decode`](Self::can_decode).
    #[must_use]
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "png" => Some(ImageFormat::Png),
            "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
            "tif" | "tiff" => Some(ImageFormat::Tiff),
            _ => None,
        }
    }

    /// The canonical IANA media type (MIME) for this format.
    #[must_use]
    pub fn mime_type(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Tiff => "image/tiff",
        }
    }

    /// Detect the format from the leading bytes' magic number.
    ///
    /// The ingestion registry resolves format from a hint (extension, content
    /// type) and only sniffs as a last resort, so most callers already know the
    /// format and pass it to [`open`](crate::ImageBuffer::open). This is for the
    /// one that genuinely has only bytes — the `#exif` sub-part reader, handed
    /// the image's own container to strip metadata from.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the bytes are not a recognizable image, or
    /// [`ErrorKind::CapabilityUnavailable`](elide_core::ErrorKind::CapabilityUnavailable)
    /// for a recognized format this crate does not model.
    pub fn detect(bytes: &[u8]) -> elide_core::Result<Self> {
        use elide_core::{Error, ErrorKind};
        let guessed = image::guess_format(bytes)
            .map_err(|e| Error::new(ErrorKind::MalformedInput, format!("unknown image: {e}")))?;
        match guessed {
            image::ImageFormat::Png => Ok(ImageFormat::Png),
            image::ImageFormat::Jpeg => Ok(ImageFormat::Jpeg),
            image::ImageFormat::Tiff => Ok(ImageFormat::Tiff),
            other => Err(Error::new(
                ErrorKind::CapabilityUnavailable,
                format!("unsupported image format: {other:?}"),
            )),
        }
    }

    /// Whether this build has the codec to decode and encode this format.
    ///
    /// Naming a format always works; decoding needs its format feature. `open`
    /// returns a capability error for a format where this is `false`.
    #[must_use]
    pub fn can_decode(self) -> bool {
        match self {
            ImageFormat::Png => cfg!(feature = "png"),
            ImageFormat::Jpeg => cfg!(feature = "jpeg"),
            ImageFormat::Tiff => cfg!(feature = "tiff"),
        }
    }

    /// The `image` crate's format this maps to, for the decode/encode calls.
    pub(crate) fn to_image(self) -> image::ImageFormat {
        match self {
            ImageFormat::Png => image::ImageFormat::Png,
            ImageFormat::Jpeg => image::ImageFormat::Jpeg,
            ImageFormat::Tiff => image::ImageFormat::Tiff,
        }
    }

    /// The `little_exif` file type this maps to, for the metadata calls.
    #[cfg(feature = "exif")]
    pub(crate) fn to_exif(self) -> little_exif::filetype::FileExtension {
        use little_exif::filetype::FileExtension;
        match self {
            ImageFormat::Png => FileExtension::PNG {
                as_zTXt_chunk: false,
            },
            ImageFormat::Jpeg => FileExtension::JPEG,
            ImageFormat::Tiff => FileExtension::TIFF,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_type_maps_each_format() {
        assert_eq!(ImageFormat::Png.mime_type(), "image/png");
        assert_eq!(ImageFormat::Jpeg.mime_type(), "image/jpeg");
        assert_eq!(ImageFormat::Tiff.mime_type(), "image/tiff");
    }

    #[test]
    fn from_extension_round_trips_a_canonical_extension() {
        assert_eq!(ImageFormat::from_extension("PNG"), Some(ImageFormat::Png));
        assert_eq!(ImageFormat::from_extension("jpeg"), Some(ImageFormat::Jpeg));
        assert_eq!(ImageFormat::from_extension("bmp"), None);
    }
}
