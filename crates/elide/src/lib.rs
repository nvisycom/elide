#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

pub mod backend;
pub mod detection;
pub mod enrichment;
pub mod modality;
pub mod primitive;
pub mod recognition;
pub mod redaction;

#[cfg(feature = "codec")]
#[cfg_attr(docsrs, doc(cfg(feature = "codec")))]
pub mod codec;

/// Re-export of [`async_trait`] for implementing the toolkit's async traits.
///
/// The public async traits (`Recognizer`, `Operator`, `Enricher`,
/// `DataReader`, …) are `#[async_trait]`, so an `impl` block must carry the
/// attribute. Use this re-export instead of depending on `async-trait`
/// directly, the version is guaranteed to match:
///
/// ```ignore
/// #[elide::async_trait]
/// impl Recognizer<Text> for MyRecognizer { /* async fn recognize … */ }
/// ```
///
/// [`async_trait`]: async_trait::async_trait
pub use async_trait::async_trait;
#[doc(inline)]
pub use elide_core::entity;
#[doc(inline)]
pub use elide_core::{Error, ErrorKind, Result};
// The orchestration engine (`Orchestrator`, `Report`, `Directives`, …), a
// small curated set, re-exported flat at the root under the `engine` feature.
// `engine` implies `codec` (the orchestrator decodes through it) and serde.
#[cfg(feature = "engine")]
#[cfg_attr(docsrs, doc(cfg(feature = "engine")))]
#[doc(inline)]
pub use elide_engine::{
    AnalyzedDocument, ArtifactSet, AsDocuments, Directives, Document, Orchestrator, PartId,
    RegistryDocumentExt, Report, ReportDeserializer,
};

pub mod prelude;

/// The book's examples, one item per chapter, compiled as doctests. The book is
/// not published with the crate, so this only builds in the repository, and
/// only with the features its chapters use (CI tests with `--all-features`).
#[cfg(all(
    doctest,
    feature = "engine",
    feature = "mocks",
    feature = "llm-rig",
    feature = "codec-txt",
    feature = "codec-html",
    feature = "codec-png",
    feature = "codec-docx",
    feature = "codec-wav",
    feature = "metrics-tracing",
    feature = "operator-aes",
    feature = "metrics",
    feature = "ratelimit",
    feature = "routing",
))]
mod book {
    macro_rules! chapters {
        ($($name:ident: $file:literal),* $(,)?) => {
            $(
                #[doc = include_str!(concat!("../../../book/src/", $file))]
                struct $name;
            )*
        };
    }

    chapters! {
        Introduction: "introduction.md",
        Installation: "installation.md",
        FirstRedaction: "first-redaction.md",
        Concepts: "concepts.md",
        Analyzers: "analyzers.md",
        Patterns: "patterns.md",
        Ner: "ner.md",
        Llm: "llm.md",
        Reconciling: "reconciling.md",
        RecognizerDecorators: "recognizer-decorators.md",
        Rules: "rules.md",
        Operators: "operators.md",
        Reversible: "reversible.md",
        Formats: "formats.md",
        Containers: "containers.md",
        Review: "review.md",
        Metadata: "metadata.md",
        Images: "images.md",
        Audio: "audio.md",
        Backends: "backends.md",
        Usage: "usage.md",
        RateLimits: "rate-limits.md",
        Routing: "routing.md",
        Failover: "failover.md",
        CustomRecognizers: "custom-recognizers.md",
        CustomBackends: "custom-backends.md",
        Features: "features.md",
    }
}
