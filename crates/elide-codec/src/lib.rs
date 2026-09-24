#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod content;
mod contract;
#[cfg(feature = "extract")]
pub mod extract;
pub mod string;
#[cfg(feature = "test-util")]
pub mod test_util;

pub use self::contract::{
    Container, DocumentHandle, Format, FormatId, Handler, Loader, LocalId, Part,
    UntypedDocumentHandle,
};
