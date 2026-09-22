//! [`ImageFormat`]: the raster formats this crate supports.

use elide_core::{Error, ErrorKind, Result};

/// A raster image format this crate can decode, encode, and strip metadata from.
///
/// A closed set, not a re-export of [`image::ImageFormat`]: the `image` crate
/// knows many formats this crate does not handle, so a caller can only name one
/// that is actually supported, and an unsupported input format is rejected at
/// the boundary rather than failing deeper in. Each variant is present only when
/// its format feature is enabled, so a build cannot name a format it cannot
/// decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum ImageFormat {
    /// PNG.
    #[cfg(feature = "png")]
    Png,
    /// JPEG.
    #[cfg(feature = "jpeg")]
    Jpeg,
    /// TIFF.
    #[cfg(feature = "tiff")]
    Tiff,
}

impl ImageFormat {
    /// The format a filename `extension` names (case-insensitive), or `None` when
    /// it is not a format this build supports.
    ///
    /// A filename-based *hint* only: the authoritative format is what
    /// [`ImageBuffer::open`](crate::ImageBuffer::open) detects from the bytes. An
    /// extension whose format feature is not enabled maps to `None`.
    #[must_use]
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            #[cfg(feature = "png")]
            "png" => Some(ImageFormat::Png),
            #[cfg(feature = "jpeg")]
            "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
            #[cfg(feature = "tiff")]
            "tif" | "tiff" => Some(ImageFormat::Tiff),
            _ => None,
        }
    }

    /// The `image` crate's format this maps to, for the decode/encode calls.
    pub(crate) fn to_image(self) -> image::ImageFormat {
        match self {
            #[cfg(feature = "png")]
            ImageFormat::Png => image::ImageFormat::Png,
            #[cfg(feature = "jpeg")]
            ImageFormat::Jpeg => image::ImageFormat::Jpeg,
            #[cfg(feature = "tiff")]
            ImageFormat::Tiff => image::ImageFormat::Tiff,
        }
    }

    /// Map an `image` crate format to ours, rejecting one this crate does not
    /// support.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::CapabilityUnavailable`] for a format outside the supported
    /// set.
    pub(crate) fn from_image(format: image::ImageFormat) -> Result<Self> {
        match format {
            #[cfg(feature = "png")]
            image::ImageFormat::Png => Ok(ImageFormat::Png),
            #[cfg(feature = "jpeg")]
            image::ImageFormat::Jpeg => Ok(ImageFormat::Jpeg),
            #[cfg(feature = "tiff")]
            image::ImageFormat::Tiff => Ok(ImageFormat::Tiff),
            other => Err(Error::new(
                ErrorKind::CapabilityUnavailable,
                format!("unsupported image format: {other:?}"),
            )),
        }
    }

    /// The `little_exif` file type this maps to, for the metadata calls.
    #[cfg(feature = "exif")]
    pub(crate) fn to_exif(self) -> little_exif::filetype::FileExtension {
        use little_exif::filetype::FileExtension;
        match self {
            #[cfg(feature = "png")]
            ImageFormat::Png => FileExtension::PNG {
                as_zTXt_chunk: false,
            },
            #[cfg(feature = "jpeg")]
            ImageFormat::Jpeg => FileExtension::JPEG,
            #[cfg(feature = "tiff")]
            ImageFormat::Tiff => FileExtension::TIFF,
        }
    }
}
