//! Recognition: the [`Recognizer`] contract and its implementations.
//!
//! Re-exports the core recognition vocabulary from
//! [`elide_core::recognition`], and nests each shipped recognizer crate
//! behind a feature: [`pattern`], [`ner`], [`llm`]. Pre-recognition
//! passes (language detection, transcription, OCR) are [`Enricher`]s and
//! live in the [`enrichment`] module.
//!
//! [`Recognizer`]: elide_core::recognition::Recognizer
//! [`Enricher`]: elide_core::recognition::Enricher
//! [`enrichment`]: crate::enrichment
//! [`pattern`]: crate::recognition::pattern
//! [`ner`]: crate::recognition::ner
//! [`llm`]: crate::recognition::llm

// The core recognition vocabulary, minus `Enricher` — enrichers are a
// pre-recognition concern and live in the `enrichment` module.
#[doc(inline)]
pub use elide_core::recognition::{
    Artifacts, LabelMap, Recognizer, RecognizerContext, RecognizerId, Scope, annotation,
};

/// Context-enhanced recognition: keyword-boosted confidence over another
/// recognizer.
///
/// [`Enhanced`] wraps a [`Recognizer`] and runs an [`Enhancer`] (built
/// from [`BoostRule`]s) over its entities, lifting confidence where a
/// context keyword fires near an entity. This is the home of the type
/// `PatternRecognizer::build_context_enhanced` returns. Re-exported from
/// [`elide_context`].
///
/// The return type of `build_context_enhanced` is now nameable through
/// the facade, so a caller can store or return it:
///
/// ```
/// # #[cfg(feature = "pattern")] {
/// use elide::recognition::context::Enhanced;
/// use elide::recognition::pattern::PatternRecognizer;
///
/// fn build() -> Enhanced<PatternRecognizer> {
///     PatternRecognizer::builder()
///         .build_context_enhanced()
///         .expect("recognizer builds")
/// }
/// # }
/// ```
///
/// [`Recognizer`]: elide_core::recognition::Recognizer
/// [`Enhanced`]: elide_context::Enhanced
/// [`Enhancer`]: elide_context::Enhancer
/// [`BoostRule`]: elide_context::BoostRule
pub mod context {
    #[doc(inline)]
    pub use elide_context::{Boost, BoostRule, Context, Enhanced, Enhancer};
}

/// LLM-mediated recognition: prompt a language or vision model over
/// text and images.
#[cfg(feature = "llm")]
#[cfg_attr(docsrs, doc(cfg(feature = "llm")))]
#[doc(inline)]
pub use elide_llm as llm;
/// Model-based named-entity recognition: detect entities and their
/// language.
#[cfg(feature = "ner")]
#[cfg_attr(docsrs, doc(cfg(feature = "ner")))]
#[doc(inline)]
pub use elide_ner as ner;
/// Dictionary- and pattern-based recognition: match entities by regex
/// and term lists.
#[cfg(feature = "pattern")]
#[cfg_attr(docsrs, doc(cfg(feature = "pattern")))]
#[doc(inline)]
pub use elide_pattern as pattern;
