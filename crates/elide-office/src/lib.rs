#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod docx;
pub mod opc;
pub mod pptx;
pub mod xlsx;

mod error;

pub use self::error::{Error, ErrorKind, Result};
