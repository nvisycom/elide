#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod backend;
pub(crate) mod error;
pub mod provider;
mod recognition;

pub use self::provider::{AuthenticatedProvider, LlmProvider, UnauthenticatedProvider};
pub use self::recognition::{
    DefaultPrompt, FilePrompt, LlmRecognizer, LlmRecognizerBuilder, Prompt,
};
