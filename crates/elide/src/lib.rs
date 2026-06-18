#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod analyzer;
mod anonymizer;
pub mod deduplication;

// Umbrella re-exports: pull the sibling crates in under one roof so
// downstream code can depend on `elide` alone. `core` is always present;
// the recognizer and codec layers are feature-gated so a minimal
// dependant doesn't compile what it doesn't use.
#[cfg(feature = "codec")]
#[cfg_attr(docsrs, doc(cfg(feature = "codec")))]
#[doc(inline)]
pub use elide_codec as codec;
#[doc(inline)]
pub use elide_core as core;
#[doc(no_inline)]
pub use elide_core::redaction::Redactions;
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
