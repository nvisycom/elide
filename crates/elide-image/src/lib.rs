#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod buffer;
#[cfg(feature = "codec")]
pub mod codec;
#[cfg(feature = "exif")]
mod exif;
pub mod modality;
#[cfg(feature = "ocr")]
pub mod ocr;
mod policy;
pub mod primitive;
#[cfg(feature = "test-util")]
pub mod test_util;

pub use self::buffer::{ImageBuffer, RasterImage};
#[cfg(feature = "exif")]
pub use self::exif::ExifRecognizer;
// A dependency-free config value: always exported, no `exif` feature needed.
pub use self::policy::ExifPolicy;
