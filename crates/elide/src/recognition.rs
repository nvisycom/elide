//! Recognition: the [`Recognizer`] contract and the shipped recognizer
//! crates that implement it.
//!
//! Re-exports the core recognition vocabulary (the [`Recognizer`] and
//! [`Enricher`] traits, [`RecognizerContext`], hints, labels, …) from
//! [`elide_core::recognition`], and nests each shipped recognizer crate
//! under its own submodule behind a feature: [`pattern`], [`ner`],
//! [`llm`].
//!
//! [`Recognizer`]: elide_core::recognition::Recognizer
//! [`Enricher`]: elide_core::recognition::Enricher
//! [`RecognizerContext`]: elide_core::recognition::RecognizerContext

#[doc(inline)]
pub use elide_core::recognition::*;
/// LLM-mediated recognition (text NER and image VLM).
#[cfg(feature = "llm")]
#[cfg_attr(docsrs, doc(cfg(feature = "llm")))]
#[doc(inline)]
pub use elide_llm as llm;
/// Model-based named-entity recognition.
#[cfg(feature = "ner")]
#[cfg_attr(docsrs, doc(cfg(feature = "ner")))]
#[doc(inline)]
pub use elide_ner as ner;
/// Dictionary- and pattern-based recognition.
#[cfg(feature = "pattern")]
#[cfg_attr(docsrs, doc(cfg(feature = "pattern")))]
#[doc(inline)]
pub use elide_pattern as pattern;
