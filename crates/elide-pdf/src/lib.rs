#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod document;
pub mod extract;
pub mod inspect;
pub mod redact;
#[cfg(feature = "render")]
pub mod render;
pub mod text;

pub use elide_core::{Error, ErrorKind, Result};

pub use self::document::Pdf;
pub use self::extract::{Embedding, EmbeddingKind, ImageId, Issue, IssueKind};
pub use self::redact::Detection;
pub use self::text::{Address, GlyphBytes, OffsetMap, OffsetRun, StreamTarget};
