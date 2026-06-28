#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod modality;

/// Enrichment: pre-recognition passes that annotate the input.
///
/// Each [`Enricher`] runs ahead of the recognizers, resolving some property
/// onto the call that downstream stages read back — the same seam, whether
/// it detects a language, transcribes audio, or OCRs an image. Each shipped
/// enricher sits behind a feature: `lingua` (language detection), `stt`
/// (speech-to-text + the transcript enricher), and `ocr` (OCR + the
/// recognized-text enricher).
///
/// [`Enricher`]: elide_core::recognition::Enricher
pub mod enrichment {
    #[doc(inline)]
    pub use elide_core::recognition::Enricher;
    /// Language detection for language-aware recognizers and policies.
    #[cfg(feature = "lingua")]
    #[cfg_attr(docsrs, doc(cfg(feature = "lingua")))]
    #[doc(inline)]
    pub use elide_lingua as lingua;
    /// OCR backends and the enricher that runs text recognizers over the
    /// recognized image text.
    #[cfg(feature = "ocr")]
    #[cfg_attr(docsrs, doc(cfg(feature = "ocr")))]
    #[doc(inline)]
    pub use elide_ocr as ocr;
    /// Speech-to-text backends and the enricher that runs text recognizers
    /// over the transcript.
    #[cfg(feature = "stt")]
    #[cfg_attr(docsrs, doc(cfg(feature = "stt")))]
    #[doc(inline)]
    pub use elide_stt as stt;
}

/// Redaction: the "hide" engines and the strategies they apply.
///
/// The [`Anonymizer`] / [`Deanonymizer`] engines, the shipped [`operators`],
/// the [`vault`] backing (the default [`InMemoryVault`]), and the pseudonym
/// [`generator`]s, plus the core operator contract re-exported from
/// [`elide_core::operator`]. Re-exported from [`elide_redaction`].
///
/// [`Anonymizer`]: redaction::Anonymizer
/// [`Deanonymizer`]: redaction::Deanonymizer
/// [`Operator`]: elide_core::operator::Operator
/// [`operators`]: redaction::operators
/// [`vault`]: redaction::vault
/// [`InMemoryVault`]: redaction::vault::InMemoryVault
/// [`generator`]: redaction::generator
pub mod redaction {
    // The core operator contract, re-surfaced through the redaction crate.
    #[doc(inline)]
    pub use elide_core::operator::{
        LeakProfile, Operator, OperatorId, Redactions, ReversibleOperator,
    };
    #[doc(inline)]
    pub use elide_redaction::{Anonymizer, Deanonymizer, generator, operators, vault};
}

/// Detection: the [`Analyzer`] "find" engine and its deduplication layers.
///
/// The [`Analyzer`] runs the enrichers and recognizers, then reconciles
/// their findings through the [`Layer`] stages — [`calibrate`],
/// [`reconcile`], [`filter`] — each reshaping or pruning the working entity
/// set. Re-exported from [`elide_detection`].
///
/// [`Analyzer`]: detection::Analyzer
/// [`Layer`]: elide_detection::Layer
/// [`calibrate`]: elide_detection::calibrate
/// [`reconcile`]: elide_detection::reconcile
/// [`filter`]: elide_detection::filter
pub mod detection {
    #[doc(inline)]
    pub use elide_detection::{Analyzer, Layer, LayerOutput, calibrate, filter, reconcile};
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
    // The glob brings the `content` and `handler` submodules along with the
    // registry and handle types.
    #[doc(inline)]
    pub use elide_codec::*;
}

/// Recognition: the [`Recognizer`] contract and its implementations.
///
/// Re-exports the core recognition vocabulary from
/// [`elide_core::recognition`], and nests each shipped recognizer crate
/// behind a feature: [`pattern`], [`ner`], [`llm`]. Pre-recognition
/// passes (language detection, transcription, OCR) are [`Enricher`]s and
/// live in the [`enrichment`] module.
///
/// [`Recognizer`]: elide_core::recognition::Recognizer
/// [`Enricher`]: elide_core::recognition::Enricher
/// [`enrichment`]: crate::enrichment
/// [`pattern`]: recognition::pattern
/// [`ner`]: recognition::ner
/// [`llm`]: recognition::llm
pub mod recognition {
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
}

#[doc(inline)]
pub use elide_core::{Error, ErrorKind, Result};
#[doc(inline)]
pub use elide_core::{entity, primitive};
// The cross-stage orchestration engine stays at the root — unlike the
// per-stage engines (`detection::Analyzer`, `redaction::Anonymizer`), it
// drives detection *and* redaction across a whole document.
//
// `EntityGroup` is nameable so callers can state the
// `Vec<Entity<M>>: EntityGroup` bound on the orchestrator's `with_modality`
// and the report's `insert_*` methods; hidden, an implementation detail.
#[cfg(feature = "codec")]
#[doc(hidden)]
pub use elide_orchestration::EntityGroup;
#[cfg(feature = "codec")]
#[cfg_attr(docsrs, doc(cfg(feature = "codec")))]
#[doc(inline)]
pub use elide_orchestration::{Orchestrator, Report};

/// The common imports for assembling a pipeline.
///
/// A `use elide::prelude::*;` brings the engines ([`Analyzer`],
/// [`Anonymizer`], [`Deanonymizer`], and — with `codec` — `Orchestrator`
/// and the `FormatRegistry` that decodes documents),
/// the error types, the [`Recognizer`]/[`Operator`]/[`Modality`] contracts
/// and the [`Scope`] they run against, the deduplication [`Layer`]s with
/// their usual strategies, and the common vocabulary — the modality markers
/// (`Text`, and the feature-gated `Image`/`Audio`/`Tabular`), `Entity`,
/// `LabelRef`, the [`builtins`] label set, `Confidence`/`ConfidenceThreshold`,
/// and `Language`/`LanguageTag`. The [`operators`] module comes along too, so
/// `prelude::operators::*` reaches the concrete operators without the longer
/// path. The concrete recognizers and backends are left out — they vary per
/// use case and a few names collide — so import those from [`recognition`].
///
/// [`Analyzer`]: crate::detection::Analyzer
/// [`Anonymizer`]: crate::redaction::Anonymizer
/// [`Deanonymizer`]: crate::redaction::Deanonymizer
/// [`Recognizer`]: crate::recognition::Recognizer
/// [`Operator`]: crate::redaction::Operator
/// [`Modality`]: crate::modality::Modality
/// [`Scope`]: crate::recognition::Scope
/// [`Layer`]: crate::detection::Layer
/// [`builtins`]: crate::entity::builtins
/// [`operators`]: crate::redaction::operators
/// [`recognition`]: crate::recognition
pub mod prelude {
    #[cfg(feature = "codec")]
    #[doc(no_inline)]
    pub use elide_codec::FormatRegistry;
    #[doc(no_inline)]
    pub use elide_core::entity::{Entity, LabelRef, builtins};
    #[doc(no_inline)]
    pub use elide_core::modality::Modality;
    #[cfg(feature = "audio")]
    #[doc(no_inline)]
    pub use elide_core::modality::audio::Audio;
    #[cfg(feature = "image")]
    #[doc(no_inline)]
    pub use elide_core::modality::image::Image;
    #[cfg(feature = "tabular")]
    #[doc(no_inline)]
    pub use elide_core::modality::tabular::Tabular;
    #[doc(no_inline)]
    pub use elide_core::modality::text::Text;
    #[doc(no_inline)]
    pub use elide_core::primitive::{Confidence, ConfidenceThreshold, Language, LanguageTag};
    #[doc(no_inline)]
    pub use elide_core::recognition::{Recognizer, Scope};
    #[doc(no_inline)]
    pub use elide_core::{Error, ErrorKind, Result};
    #[doc(no_inline)]
    pub use elide_detection::{
        Analyzer, Layer,
        calibrate::CalibrateLayer,
        filter::FilterLayer,
        reconcile::{
            ReconcileLayer,
            group::{DiffLabelOverlap, LabelOverlap},
            reconciler::{Merging, Structural},
            scoring::Max,
        },
    };
    #[cfg(feature = "codec")]
    #[doc(no_inline)]
    pub use elide_orchestration::{Orchestrator, Report};
    #[doc(no_inline)]
    pub use elide_redaction::{Anonymizer, Deanonymizer, Operator, operators};
}
