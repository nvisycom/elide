#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod entity;
mod error;
pub mod modality;
pub mod operator;
pub mod primitive;
pub mod recognition;

pub use self::error::{Error, ErrorKind, Result};
