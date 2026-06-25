#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod entity;
mod error;
pub mod modality;
pub mod operator;
pub mod primitive;
pub mod recognition;

// The error type is the one piece flat enough to belong at the crate
// root; every other type is reached through its module path.
pub use self::error::{Error, ErrorKind, Result};
