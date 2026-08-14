#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod extract;
pub mod inspect;
pub mod redact;
#[cfg(feature = "render")]
pub mod render;

mod error;
mod pdf;

pub use self::error::{Error, ErrorKind, Result};
pub use self::pdf::Pdf;
