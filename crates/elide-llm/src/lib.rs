#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod backend;
pub mod candidates;
#[cfg(feature = "rig")]
pub(crate) mod error;
mod modality;
pub mod prompt;
#[cfg(feature = "rig")]
pub mod provider;
mod recognition;

pub use self::recognition::{LlmRecognizer, LlmRecognizerBuilder};
