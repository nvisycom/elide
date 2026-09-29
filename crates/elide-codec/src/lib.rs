#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod content;
mod contract;
#[cfg(feature = "extract")]
pub mod extract;
#[cfg(feature = "mocks")]
pub mod mocks;
pub mod string;

pub use self::contract::{
    Document, DocumentLoader, DocumentPart, EncodedPart, ErasedStream, Format, FormatId,
    LeafLoader, LeafRecombine, Loader, LocalId, Recombine, Stream, TypedStream,
};
