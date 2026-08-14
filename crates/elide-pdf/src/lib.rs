#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod block;
pub mod image;
#[cfg(feature = "render")]
pub mod render;

mod error;
mod pdf;

pub use self::error::{Error, ErrorKind, Result};
pub use self::pdf::Pdf;
