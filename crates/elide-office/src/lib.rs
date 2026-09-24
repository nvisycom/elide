#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

#[cfg(feature = "codec")]
pub mod codec;
pub mod docx;
pub mod ooxml;
pub mod opc;
pub mod pptx;
pub mod xlsx;

mod error;

pub use self::error::{Error, ErrorKind, Result};
