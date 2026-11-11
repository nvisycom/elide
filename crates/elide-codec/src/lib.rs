#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod content;
mod contract;
#[cfg(feature = "extract")]
pub mod extract;
#[cfg(feature = "metadata")]
pub mod metadata;
#[cfg(feature = "mocks")]
pub mod mocks;
pub mod string;

pub use self::contract::{
    Blob, Document, DocumentLoader, Encode, FlaggedItem, Format, FormatId, LeafDocument, Loader,
    LocalId, Native, Stream, StreamMut, Stripped, StrippedContent, TypedStream,
};
