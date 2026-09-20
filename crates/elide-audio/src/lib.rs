#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod buffer;
#[cfg(feature = "_internal")]
mod engine;
#[cfg(feature = "test-util")]
pub mod test_util;

pub use self::buffer::{AudioBuffer, AudioFormat};
