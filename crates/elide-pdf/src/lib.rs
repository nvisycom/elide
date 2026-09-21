#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod extract;
pub mod inspect;
pub mod redact;
#[cfg(feature = "render")]
pub mod render;

mod pdf;

pub use elide_core::{Error, ErrorKind, Result};

pub use self::pdf::Pdf;
