#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod buffer;
#[cfg(feature = "codec")]
pub mod codec;
#[cfg(feature = "_internal")]
mod engine;
#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;
pub mod modality;
pub mod primitive;
#[cfg(feature = "stt")]
pub mod stt;

pub use self::buffer::AudioBuffer;
