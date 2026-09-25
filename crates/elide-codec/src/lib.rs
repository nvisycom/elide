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
    Document, DocumentLoader, DocumentPart, EncodedPart, ErasedStream, Format, FormatId,
    LeafLoader, LeafRecombine, Loader, LocalId, Recombine, Stream, TypedStream,
};
