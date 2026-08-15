#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod block;
pub mod part;

mod docx;
mod error;

pub use self::docx::Docx;
pub use self::error::{Error, ErrorKind, Result};
