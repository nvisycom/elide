#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod enrichment;
pub mod entity;
mod error;
#[cfg(feature = "mocks")]
pub mod mocks;
pub mod modality;
pub mod primitive;
pub mod recognition;
pub mod redaction;

pub use self::error::{Error, ErrorKind, Result};
