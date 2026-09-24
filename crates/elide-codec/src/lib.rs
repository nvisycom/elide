#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod codec;
pub mod content;
pub mod context;
#[cfg(feature = "internal_extract")]
pub mod extract;
#[cfg(feature = "internal_text")]
pub mod redact;

pub use self::codec::{
    Container, DocumentHandle, Format, FormatId, Handler, Loader, LocalId, Part,
    UntypedDocumentHandle,
};
