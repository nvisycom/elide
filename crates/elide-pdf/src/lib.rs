#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod codec;
#[cfg(any(test, feature = "fixtures"))]
#[cfg_attr(docsrs, doc(cfg(feature = "fixtures")))]
pub mod fixtures;
pub mod primitive;

pub(crate) mod content;
pub(crate) mod pdf;
pub(crate) mod sanitize;
