#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod analyzer;
mod anonymizer;
pub mod deduplication;
pub mod recognition;
pub mod redaction;

// Core re-exports: flatten the error vocabulary to the crate root and
// surface core's domain modules directly (so callers write `elide::entity`
// rather than `elide::core::entity`). The recognition and redaction
// modules are defined locally (they also nest the shipped recognizer
// crates / the anonymizer engine); the codec layer is feature-gated so a
// minimal dependant doesn't compile what it doesn't use.
#[cfg(feature = "codec")]
#[cfg_attr(docsrs, doc(cfg(feature = "codec")))]
#[doc(inline)]
pub use elide_codec as codec;
#[doc(inline)]
pub use elide_core::{Error, ErrorKind, Result};
#[doc(inline)]
pub use elide_core::{entity, modality, primitive};

pub use self::analyzer::Analyzer;
