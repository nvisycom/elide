#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod buffer;
#[cfg(feature = "exif")]
mod exif;
#[cfg(feature = "test-util")]
pub mod test_util;

pub use self::buffer::{ImageBuffer, ImageFormat};
#[cfg(feature = "exif")]
pub use self::exif::{ExifPolicy, ExifRecognizer};
