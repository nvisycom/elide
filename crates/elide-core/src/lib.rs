#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod entity;
mod error;
pub mod modality;
pub mod operator;
pub mod primitive;
pub mod recognition;

#[cfg(feature = "test-utils")]
#[cfg_attr(docsrs, doc(cfg(feature = "test-utils")))]
pub mod test_util;

pub use self::error::{Error, ErrorKind, Result};
