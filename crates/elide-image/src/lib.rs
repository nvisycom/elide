#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod buffer;
#[cfg(feature = "codec")]
pub mod codec;
#[cfg(feature = "exif")]
#[cfg_attr(docsrs, doc(cfg(feature = "exif")))]
pub mod exif;
pub mod modality;
#[cfg(feature = "ocr")]
pub mod ocr;
pub mod primitive;
#[cfg(feature = "test-util")]
pub mod test_util;

pub use self::buffer::{ImageBuffer, RasterImage};
