//! [`ImageFormat`]: the raster formats this crate supports.

use elide_core::{Error, ErrorKind, Result};

/// A raster image format this crate can decode, encode, and (for JPEG/PNG)
/// strip metadata from.
///
/// A closed set, not a re-export of [`image::ImageFormat`]: the `image` crate
/// knows many formats this crate does not handle, so a caller can only name one
/// that is actually supported, and an unsupported input format is rejected at
/// the boundary rather than failing deeper in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum ImageFormat {
    /// PNG.
    Png,
    /// JPEG.
    Jpeg,
}

impl ImageFormat {
    /// The `image` crate's format this maps to, for the decode/encode calls.
    pub(crate) fn to_image(self) -> image::ImageFormat {
        match self {
            ImageFormat::Png => image::ImageFormat::Png,
            ImageFormat::Jpeg => image::ImageFormat::Jpeg,
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
            image::ImageFormat::Png => Ok(ImageFormat::Png),
            image::ImageFormat::Jpeg => Ok(ImageFormat::Jpeg),
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
            ImageFormat::Png => FileExtension::PNG {
                as_zTXt_chunk: false,
            },
            ImageFormat::Jpeg => FileExtension::JPEG,
        }
    }
}
