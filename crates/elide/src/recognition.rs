//! Recognition: the [`Recognizer`] contract and its implementations.
//!
//! Re-exports the core recognition vocabulary from
//! [`elide_core::recognition`], and nests each shipped recognizer crate
//! behind a feature: [`pattern`], [`ner`], [`llm`]. Pre-recognition
//! passes (language detection, transcription, OCR) are [`Enricher`]s and
//! live in the [`enrichment`] module.
//!
//! [`Recognizer`]: elide_core::recognition::Recognizer
//! [`Enricher`]: elide_core::enrichment::Enricher
//! [`enrichment`]: crate::enrichment
//! [`pattern`]: crate::recognition::pattern
//! [`ner`]: crate::recognition::ner
//! [`llm`]: crate::recognition::llm

// The core recognition vocabulary, minus `Enricher`, enrichers are a
// pre-recognition concern and live in the `enrichment` module.
#[doc(inline)]
pub use elide_core::recognition::{
    Context, LabelMap, Recognizer, Scope, ScopeMetadata, Subject, annotation,
};

/// Recognizer decorators: wrap a [`Recognizer`] to change *how* it runs without
/// changing *what* it detects.
///
/// Each is itself a [`Recognizer`], so they drop in anywhere one goes and
/// compose with each other:
///
/// - [`ResilientRecognizer`] turns the inner recognizer's errors into empty results
///   (logged), so one failing recognizer degrades coverage instead of aborting
///   the analysis — the opposite of the engine's fail-fast default, opted into
///   per recognizer.
/// - [`ChunkedRecognizer`] windows an over-long text with a [`Chunking`]
///   strategy so a length-limited recognizer sees all of it, rebasing the
///   per-window findings back onto the full text. [`WordWindows`] is the
///   default strategy; [`CharWindows`] bounds by character count (a tighter
///   proxy for a token limit — [`CharWindows::for_gliner`] is the conservative
///   512-token setting).
///
/// Re-exported from [`elide_detection`].
///
/// [`Recognizer`]: elide_core::recognition::Recognizer
/// [`ResilientRecognizer`]: elide_detection::ResilientRecognizer
/// [`ChunkedRecognizer`]: elide_detection::ChunkedRecognizer
/// [`Chunking`]: elide_detection::Chunking
/// [`WordWindows`]: elide_detection::WordWindows
/// [`CharWindows`]: elide_detection::CharWindows
/// [`CharWindows::for_gliner`]: elide_detection::CharWindows::for_gliner
pub mod decorator {
    #[doc(inline)]
    pub use elide_detection::{
        CharWindows, ChunkedRecognizer, Chunking, ResilientRecognizer, Window, WordWindows,
    };
}

/// Escalating recognition: run a cheap recognizer always, a costly one only when
/// a [`Decider`] judges it warranted.
///
/// [`EscalatingRecognizer`] wraps a baseline recognizer (always run) and an
/// escalated one (a hosted LLM/VLM, say), running the latter only on the
/// subjects a [`Decider`] — e.g. [`ConfidenceDecider`] — picks from what the
/// baseline found. It is itself a [`Recognizer`]. Re-exported from
/// [`elide_detection`].
///
/// [`Recognizer`]: elide_core::recognition::Recognizer
/// [`EscalatingRecognizer`]: elide_detection::escalation::EscalatingRecognizer
/// [`Decider`]: elide_detection::escalation::Decider
/// [`ConfidenceDecider`]: elide_detection::escalation::ConfidenceDecider
pub mod escalation {
    #[doc(inline)]
    pub use elide_detection::escalation::*;
}

/// LLM-mediated recognition: prompt a language or vision model over
/// text and images.
#[cfg(feature = "llm")]
#[cfg_attr(docsrs, doc(cfg(feature = "llm")))]
pub mod llm {
    #[doc(inline)]
    pub use elide_llm::*;
}
/// Model-based named-entity recognition: detect entities and their
/// language.
#[cfg(feature = "ner")]
#[cfg_attr(docsrs, doc(cfg(feature = "ner")))]
pub mod ner {
    #[doc(inline)]
    pub use elide_ner::*;
}
/// Dictionary- and pattern-based recognition: match entities by regex
/// and term lists.
#[cfg(feature = "pattern")]
#[cfg_attr(docsrs, doc(cfg(feature = "pattern")))]
pub mod pattern {
    #[doc(inline)]
    pub use elide_pattern::*;
}

/// EXIF metadata recognition: surface an image's privacy-relevant EXIF fields
/// as `Entity<Metadata>` values.
///
/// Wire it into a `Metadata` pipeline to strip an image's metadata alongside its
/// pixels: `.with_modality::<Metadata>(Analyzer::new().with_recognizer(exif::ExifRecognizer),
/// Anonymizer::new().with(Rule::fallback(Erase)))`.
#[cfg(any(feature = "codec-png", feature = "codec-jpeg", feature = "codec-tiff"))]
#[cfg_attr(
    docsrs,
    doc(cfg(any(feature = "codec-png", feature = "codec-jpeg", feature = "codec-tiff")))
)]
pub mod exif {
    #[doc(inline)]
    pub use elide_image::exif::ExifRecognizer;
}

/// OOXML document-property recognition: surface a Word/PowerPoint/Excel
/// document's `docProps` fields (author, last editor, company, timestamps) as
/// `Entity<Metadata>`.
///
/// Wire it into a `Metadata` pipeline to strip document properties alongside the
/// body text.
#[cfg(any(feature = "codec-docx", feature = "codec-pptx", feature = "codec-xlsx"))]
#[cfg_attr(
    docsrs,
    doc(cfg(any(feature = "codec-docx", feature = "codec-pptx", feature = "codec-xlsx")))
)]
pub mod docprops {
    #[doc(inline)]
    pub use elide_office::codec::DocPropsRecognizer;
}
