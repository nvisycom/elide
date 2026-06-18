#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod analyzer;
mod anonymizer;
pub mod deduplication;

// Core re-exports: flatten the error vocabulary to the crate root and
// surface core's domain modules directly (so callers write `elide::entity`
// rather than `elide::core::entity`). This re-exports core wholesale for
// now; the surface will be pruned to what's actually needed later.
// Recognizer and codec layers are feature-gated so a minimal dependant
// doesn't compile what it doesn't use.
#[cfg(feature = "codec")]
#[cfg_attr(docsrs, doc(cfg(feature = "codec")))]
#[doc(inline)]
pub use elide_codec as codec;
#[doc(no_inline)]
pub use elide_core::{Error, ErrorKind, Result};
#[doc(inline)]
pub use elide_core::{entity, modality, primitive, provenance, recognition, redaction};
#[cfg(feature = "llm")]
#[cfg_attr(docsrs, doc(cfg(feature = "llm")))]
#[doc(inline)]
pub use elide_llm as llm;
#[cfg(feature = "ner")]
#[cfg_attr(docsrs, doc(cfg(feature = "ner")))]
#[doc(inline)]
pub use elide_ner as ner;
#[cfg(feature = "pattern")]
#[cfg_attr(docsrs, doc(cfg(feature = "pattern")))]
#[doc(inline)]
pub use elide_pattern as pattern;

pub use self::analyzer::Analyzer;
pub use self::anonymizer::{Anonymizer, operators};
