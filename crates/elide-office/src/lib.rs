#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod codec;
#[cfg(any(test, feature = "fixtures"))]
#[cfg_attr(docsrs, doc(cfg(feature = "fixtures")))]
pub mod fixtures;

pub(crate) mod opc;

pub(crate) mod docx;
pub(crate) mod ooxml;
pub(crate) mod pptx;
pub(crate) mod xlsx;
