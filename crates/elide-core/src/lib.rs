#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

// Lets the derives, which name `::elide_core`, be used inside this crate.
extern crate self as elide_core;

#[cfg(feature = "fixtures")]
mod asserts;
pub mod backend;
mod component;
pub mod enrichment;
pub mod entity;
mod error;
#[cfg(feature = "mocks")]
pub mod mocks;
pub mod modality;
pub mod primitive;
pub mod recognition;
pub mod redaction;

pub use self::component::Component;
pub use self::error::{Error, ErrorKind, Result};
