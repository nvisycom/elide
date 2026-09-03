//! The shared orchestrator scaffolding for the e2e tests: [`TestOrchestrator`]
//! (an opinionated orchestrator builder) and the analyzer/anonymizer builders it
//! and the tests compose from, plus the [`docx`] fixture-bytes helper.
//!
//! [`TestOrchestrator`] wraps a partially-built [`Orchestrator`] with strong test
//! defaults, the single construction path the pipeline tests and the [`Fixture`]
//! round-trip driver share.
//!
//! [`Fixture`]: super::fixture::Fixture

use elide::codec::{DocumentHandle, FormatRegistry};
use elide::detection::Analyzer;
use elide::detection::filter::FilterLayer;
use elide::detection::reconcile::{Merging, ReconcileLayer, Structural};
use elide::entity::{Entity, LabelCatalog, builtins};
#[cfg(feature = "image")]
use elide::modality::image::Image;
use elide::modality::text::Text;
use elide::modality::{DataReader, DataWriter, Modality, StreamDataReader, TextRecognizable};
use elide::primitive::ConfidenceThreshold;
use elide::recognition::Scope;
use elide::recognition::pattern::PatternRecognizer;
use elide::redaction::operators::{Erase, Mask, Replace};
use elide::redaction::{Anonymizer, Operator, Rule};
use elide::{Orchestrator, Result};

// ---- Fixture builders --------------------------------------------------------

/// Build the detection side: the real built-in pattern recognizer (with
/// context boosting) plus, when the `ner` feature is on, the NER
/// recognizer, behind the standard dedup pipeline. Generic over any
/// text-payload modality the recognizers serve.
///
/// The NER recognizer rides a mock backend that finds nothing today, so
/// name/organization/address fixtures round-trip un-redacted for now; the
/// pipeline shape is already the intended one, so those fixtures light up
/// unchanged once a real backend is configured.
pub fn build_analyzer<M: TextRecognizable>() -> Result<Analyzer<M>> {
    let patterns = PatternRecognizer::builder()
        .with_builtin_patterns()
        .with_builtin_dictionaries()
        .build_context_enhanced()?;

    let analyzer = Analyzer::new();

    // Detect the document's languages first (when `lingua` is on), so a
    // per-language context rule, a credit card beside `tarjeta` / `carte` /
    // `Kreditkarte`, fires under whatever language that sentence is in. With no
    // caller-asserted language, the enricher's detections are authoritative.
    #[cfg(feature = "lingua")]
    let analyzer = {
        use elide::enrichment::lingua::LinguaEnricher;
        analyzer.with_enricher(LinguaEnricher::unrestricted())
    };

    let analyzer = analyzer.with_recognizer(patterns);

    #[cfg(feature = "ner")]
    let analyzer = {
        use elide::recognition::ner::NerRecognizer;
        let ner = NerRecognizer::builder()
            .with_name("mock-ner")
            .with_mock_backend()
            .build()?;
        analyzer.with_recognizer(ner)
    };

    Ok(analyzer
        .with_layer(ReconcileLayer::same_label(Merging::max()))
        .with_layer(ReconcileLayer::cross_label(Structural::default()))
        .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE)))
}

/// The default [`Text`] analyzer used by [`TestOrchestrator`] and [`Fixture`]:
/// [`build_analyzer`] plus, when `llm` is on, the mock LLM recognizer. The LLM
/// recognizer is bound to [`LlmModality`], which `Text` satisfies but `Tabular`
/// does not (see `testdata/BUGS.md` B9), so only `Text` pipelines carry it today;
/// a container's tabular body still drives its text sub-parts through this `Text`
/// pipeline.
///
/// Like the mock NER, the mock LLM finds nothing today; it makes the pipeline
/// shape the intended one, so LLM-tier fixtures light up unchanged once a real
/// backend is configured.
///
/// [`Fixture`]: super::fixture::Fixture
#[cfg(feature = "llm")]
pub fn default_text_analyzer() -> Result<Analyzer<Text>> {
    use elide::recognition::llm::LlmRecognizer;
    let llm = LlmRecognizer::builder()
        .with_name("mock-llm")
        .with_mock_backend()
        .with_default_prompt()
        .build()?;
    Ok(build_analyzer::<Text>()?.with_recognizer(llm))
}

/// Without the `llm` feature the default text analyzer is just [`build_analyzer`]
/// (no LLM tier), so the pipeline shape is identical modulo the mock LLM that
/// finds nothing anyway.
#[cfg(not(feature = "llm"))]
pub fn default_text_analyzer() -> Result<Analyzer<Text>> {
    build_analyzer::<Text>()
}

/// Build the redaction side: `Replace::default()` (`[{label}]`) as the fallback,
/// so every detected label redacts to its own `[<label_id>]` token (e.g.
/// `[email_address]`, `[phone_number]`) that assertions can spot, with payment
/// cards masked instead, since a masked card is what those tests check.
pub fn build_anonymizer<M: TextRecognizable>() -> Anonymizer<M>
where
    Replace: Operator<M>,
    Mask: Operator<M>,
{
    Anonymizer::new()
        .with(Rule::label(builtins::PAYMENT_CARD.to_ref(), Mask::stars()))
        .with(Rule::fallback(Replace::default()))
}

/// An erase-everything [`Anonymizer`] for a modality `M`, the image default, and
/// what OCR image tests use.
pub fn erase_anonymizer<M: Modality>() -> Anonymizer<M>
where
    Erase: Operator<M>,
{
    Anonymizer::new().with(Rule::fallback(Erase))
}

/// The default image [`Analyzer`]: a mock LLM recognizer that detects nothing,
/// enough to register the image pipeline so a container's embedded media is
/// driven, without an OCR backend. Image tests that need real detection override
/// with [`ocr_analyzer`] via [`TestOrchestrator::with_image`].
#[cfg(all(feature = "image", feature = "llm"))]
pub fn image_analyzer() -> Result<Analyzer<Image>> {
    use elide::recognition::llm::LlmRecognizer;
    Ok(Analyzer::new().with_recognizer(
        LlmRecognizer::<Image>::builder()
            .with_name("mock-image")
            .with_mock_backend()
            .with_default_prompt()
            .build()?,
    ))
}

/// An image [`Analyzer`] that enriches with OCR from `backend` (a mock returning
/// canned layout blocks) and detects built-in patterns over the recognized text.
/// The image override for the OCR tests.
#[cfg(feature = "ocr")]
pub fn ocr_analyzer(backend: elide::enrichment::ocr::MockBackend) -> Result<Analyzer<Image>> {
    use elide::enrichment::ocr::OcrEnricher;
    Ok(Analyzer::new()
        .with_enricher(
            OcrEnricher::builder()
                .with_name("mock-ocr")
                .with_backend(backend)
                .build()?,
        )
        .with_recognizer(
            PatternRecognizer::builder()
                .with_builtin_patterns()
                .build()?,
        ))
}

// ---- TestOrchestrator --------------------------------------------------------

/// An orchestrator with strong test defaults: the built-in label catalog, the
/// built-in format registry, and default Text (+ Image, with `llm`) pipelines,
/// so a test wanting the standard setup writes `TestOrchestrator::new()?.build()`.
/// Override any piece with the `with_*` methods, or start from
/// [`bare`](Self::bare) (scope + registry, no pipelines) and add only the
/// modalities the test needs.
///
/// Wraps a partially-built [`Orchestrator`] and applies each modality eagerly, so
/// [`with_body`](Self::with_body) can register *any* modality (`Text`, `Tabular`,
/// `Image`), the generic arm [`Fixture`] uses for a `Tabular` body.
///
/// Defaults ([`new`](Self::new)):
/// - scope: [`LabelCatalog::with_builtins`] (detect every built-in).
/// - registry: [`FormatRegistry::with_builtin`].
/// - Text: [`default_text_analyzer`]; [`build_anonymizer`] (`[{label}]` fallback,
///   card masked).
/// - Image (with `image`+`llm`): a mock-LLM recognizer that detects nothing;
///   erase anonymizer.
///
/// [`Fixture`]: super::fixture::Fixture
pub struct TestOrchestrator {
    inner: Orchestrator,
}

impl TestOrchestrator {
    /// Scope (built-in catalog) + registry (built-in), with **no** modality
    /// pipelines. Opt in to each modality with [`with_body`](Self::with_body) /
    /// [`with_text`](Self::with_text) / [`with_image`](Self::with_image).
    #[must_use]
    pub fn bare() -> Self {
        Self {
            inner: Orchestrator::new()
                .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
                .with_registry(FormatRegistry::with_builtin()),
        }
    }

    /// The fully opinionated default (see the type docs): [`bare`](Self::bare)
    /// plus the default Text pipeline and, with `image`+`llm`, the default
    /// Image pipeline. Errors only if a default recognizer fails to build.
    pub fn new() -> Result<Self> {
        let this = Self::bare().with_text(default_text_analyzer()?, build_anonymizer::<Text>());
        #[cfg(all(feature = "image", feature = "llm"))]
        let this = this.with_image(image_analyzer()?, erase_anonymizer());
        Ok(this)
    }

    /// Swap the format registry (e.g. a clone the test also holds, or a narrower
    /// one).
    #[must_use]
    pub fn with_registry(mut self, registry: FormatRegistry) -> Self {
        self.inner = self.inner.with_registry(registry);
        self
    }

    /// Swap the request [`Scope`] wholesale.
    #[must_use]
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.inner = self.inner.with_scope(scope);
        self
    }

    /// Register (or replace) the pipeline for **any** modality `M`, the generic
    /// arm. [`with_text`](Self::with_text) / [`with_image`](Self::with_image) are
    /// typed conveniences over this.
    #[must_use]
    pub fn with_body<M>(mut self, analyzer: Analyzer<M>, anonymizer: Anonymizer<M>) -> Self
    where
        M: Modality,
        Vec<Entity<M>>: serde::Serialize + serde::de::DeserializeOwned,
        M::Artifact: serde::Serialize + serde::de::DeserializeOwned,
        DocumentHandle<M>: StreamDataReader<M> + DataReader<M> + DataWriter<M>,
    {
        self.inner = self.inner.with_modality::<M>(analyzer, anonymizer);
        self
    }

    /// Register (or replace) the Text pipeline.
    #[must_use]
    pub fn with_text(self, analyzer: Analyzer<Text>, anonymizer: Anonymizer<Text>) -> Self {
        self.with_body::<Text>(analyzer, anonymizer)
    }

    /// Register (or replace) the Image pipeline.
    #[cfg(feature = "image")]
    #[must_use]
    pub fn with_image(self, analyzer: Analyzer<Image>, anonymizer: Anonymizer<Image>) -> Self {
        self.with_body::<Image>(analyzer, anonymizer)
    }

    /// Assemble the real [`Orchestrator`].
    #[must_use]
    pub fn build(self) -> Orchestrator {
        self.inner
    }
}
