#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod analyzer;
mod anonymizer;
mod deanonymizer;
pub mod deduplication;
pub mod modality;
#[cfg(feature = "codec")]
mod orchestrator;
pub mod redaction;

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

    /// Context-enhanced recognition: wrap a stream recognizer to boost
    /// confidence from nearby keywords before lifting matches.
    ///
    /// A [`StreamRecognizer`] finds matches over the recognized-text stream
    /// and returns [`EntityDraft`]s; [`Enhanced`] adapts one into a full
    /// [`Recognizer`], optionally running an [`Enhancer`] (built from
    /// [`BoostRule`]s) over the drafts first. This is the home of the type
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
    /// [`StreamRecognizer`]: elide_context::StreamRecognizer
    /// [`EntityDraft`]: elide_context::EntityDraft
    /// [`Enhanced`]: elide_context::Enhanced
    /// [`Enhancer`]: elide_context::Enhancer
    /// [`BoostRule`]: elide_context::BoostRule
    pub mod context {
        #[doc(inline)]
        pub use elide_context::{
            Boost, BoostRule, Context, DraftEvent, Enhanced, Enhancer, EntityDraft,
            StreamRecognizer, lift,
        };
    }

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
    /// OCR backends and the enricher that drives the text recognizers over
    /// recognized image text.
    #[cfg(feature = "ocr")]
    #[cfg_attr(docsrs, doc(cfg(feature = "ocr")))]
    #[doc(inline)]
    pub use elide_ocr as ocr;
    /// Dictionary- and pattern-based recognition.
    #[cfg(feature = "pattern")]
    #[cfg_attr(docsrs, doc(cfg(feature = "pattern")))]
    #[doc(inline)]
    pub use elide_pattern as pattern;
    /// Speech-to-text backends and the transcript-streaming reader that
    /// drives the text recognizers over audio.
    #[cfg(feature = "stt")]
    #[cfg_attr(docsrs, doc(cfg(feature = "stt")))]
    #[doc(inline)]
    pub use elide_stt as stt;
}

#[doc(inline)]
pub use elide_core::{Error, ErrorKind, Result};
#[doc(inline)]
pub use elide_core::{entity, primitive};

pub use self::analyzer::Analyzer;
pub use self::anonymizer::Anonymizer;
pub use self::deanonymizer::Deanonymizer;
// Nameable so callers can state the `Vec<Entity<M>>: EntityGroup` bound on
// the orchestrator's construction methods; hidden, an implementation detail.
#[cfg(feature = "codec")]
#[doc(hidden)]
pub use self::orchestrator::EntityGroup;
#[cfg(feature = "codec")]
pub use self::orchestrator::{Orchestrator, Report};
