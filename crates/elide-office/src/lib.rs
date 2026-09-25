#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod codec;
pub mod opc;

pub(crate) mod docx;
pub(crate) mod ooxml;
pub(crate) mod pptx;
pub(crate) mod xlsx;

pub use elide_core::{Error, ErrorKind, Result};
