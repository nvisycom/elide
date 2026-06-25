#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod modality;
#[cfg(feature = "codec")]
mod orchestrator;

/// Redaction: the [`Operator`] contract and the strategies that apply it.
///
/// The shipped [`operators`], the default [`InMemoryVault`], the pseudonym
/// [`generator`]s, and (feature `crypto`) the [`key_provider`]s, plus the
/// core redaction vocabulary re-exported from [`elide_core::operator`]. The
/// [`Anonymizer`] / [`Deanonymizer`] engines that drive them are at the
/// crate root. Re-exported from [`elide_redaction`].
///
/// [`Operator`]: elide_core::operator::Operator
/// [`operators`]: redaction::operators
/// [`InMemoryVault`]: redaction::InMemoryVault
/// [`generator`]: redaction::generator
/// [`key_provider`]: redaction::key_provider
/// [`Anonymizer`]: crate::Anonymizer
/// [`Deanonymizer`]: crate::Deanonymizer
pub mod redaction {
    #[doc(inline)]
    pub use elide_redaction::redaction::*;
}

/// Deduplication: the [`Layer`] stages that reconcile detected entities.
///
/// [`calibrate`], [`fuse`], [`resolve`], and [`filter`] each reshape or
/// prune the working entity set; an [`Analyzer`] runs them in order after
/// detection. Re-exported from [`elide_detection`].
///
/// [`Analyzer`]: crate::Analyzer
/// [`Layer`]: elide_detection::Layer
/// [`calibrate`]: elide_detection::calibrate
/// [`fuse`]: elide_detection::fuse
/// [`resolve`]: elide_detection::resolve
/// [`filter`]: elide_detection::filter
pub mod deduplication {
    #[doc(inline)]
    pub use elide_detection::{Layer, LayerOutput, calibrate, filter, fuse, resolve};
}

/// Codec: decode documents into modality payloads, then re-encode them.
///
/// Format handlers (text, JSON, HTML, images, audio, …) sit behind a
/// [`FormatRegistry`]: each turns raw bytes into something recognizers
/// and operators can address, then folds the redactions back into the
/// original container. Re-exported from [`elide_codec`].
///
/// [`FormatRegistry`]: elide_codec::FormatRegistry
#[cfg(feature = "codec")]
#[cfg_attr(docsrs, doc(cfg(feature = "codec")))]
pub mod codec {
    #[doc(inline)]
    pub use elide_codec::*;
    #[doc(inline)]
    pub use elide_codec::{content, handler};
}

/// Recognition: the [`Recognizer`] contract and its implementations.
///
/// Re-exports the core recognition vocabulary from
/// [`elide_core::recognition`], and nests each shipped recognizer crate
/// behind a feature: [`pattern`], [`ner`], [`llm`].
///
/// [`Recognizer`]: elide_core::recognition::Recognizer
/// [`pattern`]: recognition::pattern
/// [`ner`]: recognition::ner
/// [`llm`]: recognition::llm
pub mod recognition {
    #[doc(inline)]
    pub use elide_core::recognition::*;

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

    /// Language detection: resolve the language(s) of a piece of text for
    /// language-aware recognizers and policies.
    #[cfg(feature = "lingua")]
    #[cfg_attr(docsrs, doc(cfg(feature = "lingua")))]
    #[doc(inline)]
    pub use elide_lingua as lingua;
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
    /// OCR backends and the enricher that runs text recognizers over the
    /// recognized image text.
    #[cfg(feature = "ocr")]
    #[cfg_attr(docsrs, doc(cfg(feature = "ocr")))]
    #[doc(inline)]
    pub use elide_ocr as ocr;
    /// Dictionary- and pattern-based recognition: match entities by regex
    /// and term lists.
    #[cfg(feature = "pattern")]
    #[cfg_attr(docsrs, doc(cfg(feature = "pattern")))]
    #[doc(inline)]
    pub use elide_pattern as pattern;
    /// Speech-to-text backends and the enricher that runs text recognizers
    /// over the transcript.
    #[cfg(feature = "stt")]
    #[cfg_attr(docsrs, doc(cfg(feature = "stt")))]
    #[doc(inline)]
    pub use elide_stt as stt;
}

#[doc(inline)]
pub use elide_core::{Error, ErrorKind, Result};
#[doc(inline)]
pub use elide_core::{entity, primitive};
#[doc(inline)]
pub use elide_detection::Analyzer;
#[doc(inline)]
pub use elide_redaction::{Anonymizer, Deanonymizer};

// Nameable so callers can state the `Vec<Entity<M>>: EntityGroup` bound on
// the orchestrator's construction methods; hidden, an implementation detail.
#[cfg(feature = "codec")]
#[doc(hidden)]
pub use self::orchestrator::EntityGroup;
#[cfg(feature = "codec")]
pub use self::orchestrator::{Orchestrator, Report};
