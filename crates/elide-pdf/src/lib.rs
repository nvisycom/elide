#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod codec;
pub mod primitive;

pub(crate) mod attachment;
pub(crate) mod content;
pub(crate) mod document;
pub(crate) mod error;
pub(crate) mod extract;
pub(crate) mod image;
pub(crate) mod lopdf_ext;
#[cfg(feature = "render")]
pub(crate) mod render;
pub(crate) mod sanitize;
pub(crate) mod text;

pub use elide_core::{Error, ErrorKind, Result};
