#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod buffer;
#[cfg(feature = "codec")]
pub mod codec;
#[cfg(feature = "_internal")]
mod engine;
pub mod modality;
pub mod primitive;
#[cfg(feature = "stt")]
pub mod stt;
#[cfg(any(test, feature = "test-util"))]
pub mod test_util;

pub use self::buffer::AudioBuffer;
