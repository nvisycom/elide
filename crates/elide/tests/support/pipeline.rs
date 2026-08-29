//! The shared codec round-trip driver for the e2e tests.
//!
//! Wires the same flow the `redact-txt` example does — decode (codec) →
//! analyze (`Analyzer`) → anonymize (`Anonymizer`) → encode — into one
//! helper the per-format tests call, plus a [`PipelineOutcome`] carrying
//! the entities and re-encoded bytes for assertions.
//!
//! Generic over any [`TextRecognizable`]: the [`Text`] formats (`txt`,
//! `json`, `html`) and [`Tabular`] (`csv`). The shipped pattern
//! recognizer and the operators serve both — only the codec handle's
//! modality differs.

use elide::codec::{DocumentHandle, FormatRegistry, UntypedDocumentHandle};
use elide::detection::Analyzer;
use elide::detection::filter::FilterLayer;
use elide::detection::reconcile::{Merging, ReconcileLayer, Structural};
use elide::entity::{Entity, Label, LabelCatalog, builtins};
#[cfg(feature = "stt")]
use elide::modality::audio::Audio;
#[cfg(any(feature = "llm", feature = "ocr"))]
use elide::modality::image::Image;
#[cfg(feature = "tabular")]
use elide::modality::tabular::Tabular;
use elide::modality::text::Text;
use elide::modality::{Modality, StreamDataReader, TextRecognizable};
use elide::primitive::{ConfidenceThreshold, Language, LanguageTag};
use elide::recognition::pattern::PatternRecognizer;
use elide::recognition::{Recognizer, Scope};
use elide::redaction::operators::{Erase, Mask, Replace};
use elide::redaction::{Anonymizer, Operator, Rule};
use elide::{Directives, EntityGroup, Error, ErrorKind, Orchestrator, Report, Result};

/// Outcome of one end-to-end run: the entities that survived dedup and
/// the re-encoded redacted document.
pub struct PipelineOutcome<M: Modality> {
    /// Entities detected and reconciled, in source coordinates. The
    /// pre-redaction snapshot, taken before `anonymize_with` runs.
    pub entities: Vec<Entity<M>>,
    /// The body entities recovered from the report `anonymize_with`
    /// returns: the same entities, now each carrying a redaction event in
    /// its provenance. Empty when no body pipeline ran.
    pub audited: Vec<Entity<M>>,
    /// Re-encoded document after redaction, as raw bytes. For text
    /// formats this is UTF-8 — use [`redacted_text`];
    /// for container formats (DOCX) it is the rebuilt package — use
    /// [`part`] to read one entry.
    ///
    /// [`redacted_text`]: Self::redacted_text
    /// [`part`]: Self::part
    pub redacted: Vec<u8>,
}

impl<M: Modality> PipelineOutcome<M> {
    /// The redacted output decoded as UTF-8 text. Panics if it is not
    /// (i.e. for a binary container format — read a [`part`]).
    ///
    /// [`part`]: Self::part
    pub fn redacted_text(&self) -> String {
        String::from_utf8(self.redacted.clone()).expect("redacted output is UTF-8 text")
    }

    /// Read one entry out of the redacted output, treating it as an OPC
    /// package (DOCX/XLSX). Returns the entry bytes decompressed, or `None`
    /// if absent. The OOXML packaging lives in `elide-office`, so the read
    /// goes through its part reader rather than reconstructing the zip here.
    pub fn part(&self, name: &str) -> Option<Vec<u8>> {
        elide_office::opc::test_util::read_part(&self.redacted, name)
    }

    /// Every text-bearing part of the redacted package (OOXML container
    /// formats), each *decompressed* to `(name, text)`.
    ///
    /// Decompressing is what makes a leak scan sound: the archive stores
    /// each part deflated, so a plaintext value can straddle the deflate
    /// stream and a substring check over the *raw* container bytes would
    /// give a false pass. `elide-office` owns this — read the parts back
    /// out, then scan.
    pub fn text_parts(&self) -> Vec<(String, String)> {
        elide_office::opc::test_util::text_parts(&self.redacted)
    }

    /// Assert that no value in `pii` survives in any text-bearing part of
    /// the redacted package — the end-to-end leak guarantee. Scans the
    /// decompressed [`text_parts`], so it holds against the real part bytes,
    /// not the deflated container.
    ///
    /// [`text_parts`]: Self::text_parts
    pub fn assert_no_pii(&self, pii: &[&str]) {
        for (name, text) in self.text_parts() {
            for value in pii {
                assert!(
                    !text.contains(value),
                    "PII `{value}` survived in part `{name}`",
                );
            }
        }
    }
}

/// Build the detection side: the real built-in pattern recognizer (with
/// context boosting) plus — when the `ner` feature is on — the NER
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
    // per-language context rule — a credit card beside `tarjeta` / `carte` /
    // `Kreditkarte` — fires under whatever language that sentence is in. With no
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

/// The [`Text`] analyzer with the mock LLM recognizer added on top of
/// [`build_analyzer`]'s pattern + NER. The LLM recognizer is bound to
/// [`LlmModality`], which `Text` satisfies but `Tabular` does not (see
/// `testdata/BUGS.md` B9), so only `Text` pipelines can carry it today; a
/// container's tabular body still drives its text sub-parts through this
/// `Text` pipeline.
///
/// Like the mock NER, the mock LLM finds nothing today — it makes the
/// pipeline shape the intended one, so LLM-tier fixtures light up unchanged
/// once a real backend is configured.
#[cfg(feature = "llm")]
pub fn build_text_analyzer() -> Result<Analyzer<Text>> {
    use elide::recognition::llm::LlmRecognizer;
    let llm = LlmRecognizer::builder()
        .with_name("mock-llm")
        .with_mock_backend()
        .with_default_prompt()
        .build()?;
    Ok(build_analyzer::<Text>()?.with_recognizer(llm))
}

/// Build the redaction side: one operator per label the shipped patterns
/// emit, so assertions can spot the replacement tokens, plus a fallback.
pub fn build_anonymizer<M: TextRecognizable>() -> Anonymizer<M>
where
    Replace: Operator<M>,
    Mask: Operator<M>,
    Erase: Operator<M>,
{
    Anonymizer::new()
        .with(Rule::label(
            builtins::EMAIL_ADDRESS.to_ref(),
            Replace::new("[email_address]"),
        ))
        .with(Rule::label(
            builtins::PHONE_NUMBER.to_ref(),
            Replace::new("[phone_number]"),
        ))
        .with(Rule::label(builtins::IBAN.to_ref(), Replace::new("[iban]")))
        .with(Rule::label(
            builtins::GOVERNMENT_ID.to_ref(),
            Replace::new("[government_id]"),
        ))
        .with(Rule::label(
            builtins::IP_ADDRESS.to_ref(),
            Replace::new("[ip_address]"),
        ))
        .with(Rule::label(builtins::PAYMENT_CARD.to_ref(), Mask::stars()))
        .with(Rule::fallback(Erase))
}

/// Build an image analyzer backed by the mock LLM (detects nothing) — the
/// image pipeline the master orchestrator registers so a container's
/// embedded media is driven. Real image detection is a separate concern;
/// here it proves the multi-modal path runs.
#[cfg(feature = "llm")]
fn build_image_analyzer() -> Result<Analyzer<Image>> {
    use elide::recognition::llm::LlmRecognizer;
    let recognizer = LlmRecognizer::builder()
        .with_name("mock-image")
        .with_mock_backend()
        .with_default_prompt()
        .build()?;
    Ok(Analyzer::new().with_recognizer(recognizer))
}

/// A codec fixture the e2e tests load: the inlined source bytes, the
/// extension the codec registry resolves on, and the on-disk path the
/// redacted artifact is written next to.
///
/// Construct one per format with [`include_bytes!`] for `source` (so the
/// same shape serves text formats and binary containers like DOCX) and
/// the matching `testdata/` path, then call [`run`] / [`run_tabular`].
///
/// [`run`]: Self::run
/// [`run_tabular`]: Self::run_tabular
pub struct Fixture {
    /// Absolute path to the fixture on disk; the artifact writer derives
    /// its `testdata/audits` and `testdata/results` output dirs from this.
    pub path: &'static str,
    /// Fixture body the codec decodes (compile-time inlined bytes).
    pub source: &'static [u8],
    /// Extension hint the codec registry resolves on (`"txt"`, …).
    pub extension: &'static str,
}

impl Fixture {
    /// Run the pipeline as the [`Text`] modality (`txt`, `json`, `html`,
    /// and a DOCX's body), over the default built-in registry.
    pub async fn run(&self) -> Result<PipelineOutcome<Text>> {
        self.run_typed::<Text>(FormatRegistry::with_builtin()).await
    }

    /// Run the [`Text`] pipeline over a caller-supplied `registry`, so a
    /// test can swap in a customized format (e.g. the raster PDF handler
    /// via [`pdf_format_with`]).
    ///
    /// [`pdf_format_with`]: elide::codec::handler::pdf_format_with
    pub async fn run_with(&self, registry: FormatRegistry) -> Result<PipelineOutcome<Text>> {
        self.run_typed::<Text>(registry).await
    }

    /// Run the [`Text`] pipeline with the request [`LabelCatalog`] scoped to
    /// `labels`, so a test can assert that only the requested entity types are
    /// emitted (and everything else is left in place). An empty `labels`
    /// iterator would request nothing; pass the labels under test.
    pub async fn run_with_labels(
        &self,
        labels: impl IntoIterator<Item = Label>,
    ) -> Result<PipelineOutcome<Text>> {
        let scope = Scope::new().with_catalog(labels.into_iter().collect());
        self.run_typed_with::<Text>(FormatRegistry::with_builtin(), scope)
            .await
    }

    /// Run the [`Text`] pipeline with a caller-**asserted** language, so a
    /// test can exercise the soft language signal: a match from a pattern
    /// whose locale the asserted language contradicts is confidence-demoted
    /// (and typically pruned by the threshold). An assertion also suppresses
    /// language *detection* (the assertion is authoritative).
    pub async fn run_with_language(&self, language: LanguageTag) -> Result<PipelineOutcome<Text>> {
        let scope = Scope::new()
            .with_language(Language::asserted(language))
            .with_catalog(LabelCatalog::with_builtins());
        self.run_typed_with::<Text>(FormatRegistry::with_builtin(), scope)
            .await
    }

    /// Run the pipeline as the [`Tabular`] modality (`csv`).
    ///
    /// [`Tabular`]: elide::modality::tabular::Tabular
    #[cfg(feature = "tabular")]
    pub async fn run_tabular(&self) -> Result<PipelineOutcome<Tabular>> {
        self.run_typed::<Tabular>(FormatRegistry::with_builtin())
            .await
    }

    /// Run the pipeline as the [`Audio`] modality (`wav`, `mp3`).
    ///
    /// The audio path differs from the text formats: recognition reads a
    /// *transcript* an STT enricher stamps onto the call, not the codec's
    /// byte payload. Here the enricher is backed by the no-op mock STT
    /// backend, so no transcript is produced and no entities are detected —
    /// the clip round-trips through decode → analyze → anonymize → encode
    /// unchanged. That still exercises the whole audio codec + pipeline
    /// wiring end to end.
    ///
    /// [`Audio`]: elide::modality::audio::Audio
    #[cfg(feature = "stt")]
    pub async fn run_audio(&self) -> Result<PipelineOutcome<Audio>> {
        use elide::enrichment::stt::{MockBackend, SttEnricher};
        use elide::redaction::operators::{Erase, Silence};

        let registry = FormatRegistry::with_builtin();
        let mut document = UntypedDocumentHandle::new(self.decode_as::<Audio>(&registry).await?);

        // The mock STT backend transcribes nothing, so recognition finds
        // nothing; the anonymizer would silence/erase any time spans it did.
        let analyzer = Analyzer::new().with_enricher(
            SttEnricher::builder()
                .with_name("mock-stt")
                .with_backend(MockBackend)
                .build()
                .expect("stt enricher builds"),
        );
        let anonymizer = Anonymizer::new()
            .with(Rule::label(builtins::PHONE_NUMBER.to_ref(), Silence))
            .with(Rule::fallback(Erase));

        // A built-in catalog so the analyzer runs the enricher (the mock STT
        // still transcribes nothing) — the point is to drive the whole codec +
        // enricher path, not to detect.
        let orchestrator = Orchestrator::new()
            .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
            .with_registry(registry)
            .with_modality::<Audio>(analyzer, anonymizer);

        let report = orchestrator
            .analyze(&mut document, &Directives::new())
            .await?;
        let entities: Vec<Entity<Audio>> = report
            .entities::<Audio>()
            .map(|e| e.to_vec())
            .unwrap_or_default();
        self.write_entities(&report);
        let applied = orchestrator.anonymize_with(&mut document, report).await?;
        let audited: Vec<Entity<Audio>> = applied
            .entities::<Audio>()
            .map(|e| e.to_vec())
            .unwrap_or_default();

        let redacted = document.encode()?.as_bytes().to_vec();
        self.write_redacted(&redacted);
        Ok(PipelineOutcome {
            entities,
            audited,
            redacted,
        })
    }

    /// Run the pipeline as the [`Image`] modality (`png`, `jpeg`, `tiff`).
    ///
    /// Recognition reads OCR text an [`OcrEnricher`] stamps onto the call,
    /// not the codec's image bytes. Here the enricher is backed by the no-op
    /// mock OCR backend, so no text is recognized and no entities are found
    /// — the image round-trips through decode → analyze → anonymize → encode
    /// unchanged. That still exercises the whole image codec + OCR pipeline
    /// wiring end to end.
    ///
    /// [`OcrEnricher`]: elide::enrichment::ocr::OcrEnricher
    /// [`Image`]: elide::modality::image::Image
    #[cfg(feature = "ocr")]
    pub async fn run_image(&self) -> Result<PipelineOutcome<Image>> {
        use elide::enrichment::ocr::{MockBackend, OcrEnricher};
        use elide::redaction::operators::Erase;

        let registry = FormatRegistry::with_builtin();
        let mut document = UntypedDocumentHandle::new(self.decode_as::<Image>(&registry).await?);

        // The mock OCR backend recognizes nothing, so recognition finds
        // nothing; the anonymizer would clear any regions it did.
        let analyzer = Analyzer::new().with_enricher(
            OcrEnricher::builder()
                .with_name("mock-ocr")
                .with_backend(MockBackend)
                .build()
                .expect("ocr enricher builds"),
        );
        let anonymizer = Anonymizer::new().with(Rule::fallback(Erase));

        // A built-in catalog so the analyzer runs the enricher (the mock OCR
        // still recognizes nothing) — the point is to drive the whole codec +
        // enricher path, not to detect.
        let orchestrator = Orchestrator::new()
            .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
            .with_registry(registry)
            .with_modality::<Image>(analyzer, anonymizer);

        let report = orchestrator
            .analyze(&mut document, &Directives::new())
            .await?;
        let entities: Vec<Entity<Image>> = report
            .entities::<Image>()
            .map(|e| e.to_vec())
            .unwrap_or_default();
        self.write_entities(&report);
        let applied = orchestrator.anonymize_with(&mut document, report).await?;
        let audited: Vec<Entity<Image>> = applied
            .entities::<Image>()
            .map(|e| e.to_vec())
            .unwrap_or_default();

        let redacted = document.encode()?.as_bytes().to_vec();
        self.write_redacted(&redacted);
        Ok(PipelineOutcome {
            entities,
            audited,
            redacted,
        })
    }

    /// Decode this fixture as modality `M`, redact it through the master
    /// [`Orchestrator`] (body + any container parts), encode, write the
    /// `*.out.*` and entities artifacts, and return the outcome.
    /// Panics with a descriptive message on any stage failure.
    ///
    /// The orchestrator registers a pipeline for the body modality `M` and
    /// — when the `llm` feature is on — an image pipeline (mock backend) so
    /// a container fixture's embedded media is driven too. Registering the
    /// image modality is format-neutral: it only fires for a document that
    /// actually has image parts (a DOCX), and is inert for the rest.
    async fn run_typed<M>(&self, registry: FormatRegistry) -> Result<PipelineOutcome<M>>
    where
        M: TextRecognizable,
        Entity<M>: Clone,
        Vec<Entity<M>>: EntityGroup + serde::de::DeserializeOwned,
        DocumentHandle<M>: StreamDataReader<M>,
        Replace: Operator<M>,
        Mask: Operator<M>,
        Erase: Operator<M>,
    {
        // No asserted language: the analyzer's `LinguaEnricher` detects each
        // document's languages, so a multilingual fixture activates every one
        // of its languages' per-language context (asserting a single language
        // would suppress detection — see `LinguaEnricher::enrich`). The catalog
        // requests every built-in label, so recognizers emit all they find.
        let scope = Scope::new().with_catalog(LabelCatalog::with_builtins());
        self.run_typed_with::<M>(registry, scope).await
    }

    /// [`run_typed`] with a caller-supplied [`Scope`], so a test can scope the
    /// [`LabelCatalog`] (which entity types to emit), assert a language, or set
    /// other request-level state.
    ///
    /// [`run_typed`]: Self::run_typed
    async fn run_typed_with<M>(
        &self,
        registry: FormatRegistry,
        scope: Scope,
    ) -> Result<PipelineOutcome<M>>
    where
        M: TextRecognizable,
        Entity<M>: Clone,
        Vec<Entity<M>>: EntityGroup + serde::de::DeserializeOwned,
        DocumentHandle<M>: StreamDataReader<M>,
        Replace: Operator<M>,
        Mask: Operator<M>,
        Erase: Operator<M>,
    {
        let mut document = UntypedDocumentHandle::new(self.decode_as::<M>(&registry).await?);

        // One scope, shared across every modality pipeline.
        let orchestrator = Orchestrator::new()
            .with_registry(registry)
            .with_scope(scope)
            .with_modality::<M>(build_analyzer::<M>()?, build_anonymizer::<M>());
        // Drive embedded images too when the image recognizer is available.
        #[cfg(feature = "llm")]
        let orchestrator =
            orchestrator.with_modality::<Image>(build_image_analyzer()?, Anonymizer::new());
        // A container's text sub-parts (an XLSX comment or drawing, surfaced as
        // an XML part) are the Text modality, so register a Text pipeline to
        // drive them. When the body modality already is Text this re-registers
        // the same pipeline, which is a no-op; when it is Tabular it adds the
        // pipeline the container parts need.
        #[cfg(feature = "llm")]
        let text_analyzer = build_text_analyzer()?;
        #[cfg(not(feature = "llm"))]
        let text_analyzer = build_analyzer::<Text>()?;
        let orchestrator =
            orchestrator.with_modality::<Text>(text_analyzer, build_anonymizer::<Text>());

        // Two phases so the entities surface for assertions: detect, copy
        // the body entities out, then apply with no editing.
        let report = orchestrator
            .analyze(&mut document, &Directives::new())
            .await?;
        let entities: Vec<Entity<M>> = report
            .entities::<M>()
            .map(|e| e.to_vec())
            .unwrap_or_default();
        // Write the detected entities as JSON for inspection before the
        // report is consumed by `anonymize_with`.
        self.write_entities(&report);
        let applied = orchestrator.anonymize_with(&mut document, report).await?;
        // The returned report's entities now carry the redaction event —
        // the post-redaction audit trail.
        let audited: Vec<Entity<M>> = applied
            .entities::<M>()
            .map(|e| e.to_vec())
            .unwrap_or_default();

        let redacted = document.encode()?.as_bytes().to_vec();

        self.write_redacted(&redacted);
        Ok(PipelineOutcome {
            entities,
            audited,
            redacted,
        })
    }

    /// Decode this fixture's bytes and recover the [`DocumentHandle`] as
    /// modality `M`, erroring if the format resolves to a different one.
    async fn decode_as<M: Modality>(&self, registry: &FormatRegistry) -> Result<DocumentHandle<M>>
    where
        DocumentHandle<M>: StreamDataReader<M>,
    {
        let untyped = registry.decode(self.source, self.extension).await?;
        untyped.into::<M>().map_err(|_| {
            Error::new(
                ErrorKind::CapabilityUnavailable,
                format!(
                    "{} did not resolve to the {} modality",
                    self.extension,
                    M::NAME
                ),
            )
        })
    }

    /// Write the serialized detection [`Report`] to `testdata/audits/` as
    /// `{filename}.json` (e.g. `sample.docx.json`): the body and any
    /// container parts' findings, grouped by part id. Only with the `serde`
    /// feature; the whole `audits/` directory is gitignored.
    #[cfg(feature = "serde")]
    fn write_entities(&self, report: &Report) {
        let dir = self.output_dir("audits");
        let file = std::path::Path::new(self.path)
            .file_name()
            .expect("fixture has a file name");
        let out = dir.join(format!("{}.json", file.to_string_lossy()));
        let json = serde_json::to_string_pretty(report).expect("report serializes");
        std::fs::write(&out, json).unwrap_or_else(|e| panic!("write audit {}: {e}", out.display()));
    }

    /// No-op when `serde` is off.
    #[cfg(not(feature = "serde"))]
    fn write_entities(&self, _report: &Report) {}

    /// Write the redacted document to `testdata/results/` as
    /// `{stem}.out.{ext}`. The whole `results/` directory is gitignored.
    fn write_redacted(&self, redacted: &[u8]) {
        let dir = self.output_dir("results");
        let stem = std::path::Path::new(self.path)
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("fixture has a UTF-8 stem");
        let out = dir.join(format!("{stem}.out.{}", self.extension));
        std::fs::write(&out, redacted)
            .unwrap_or_else(|e| panic!("write result {}: {e}", out.display()));
    }

    /// The `testdata/<kind>/` directory for generated artifacts, created if
    /// absent. `kind` is `audits` (serialized reports) or `results`
    /// (redacted documents); both sit beside the input fixtures and are
    /// gitignored so a test run never dirties the tree.
    fn output_dir(&self, kind: &str) -> std::path::PathBuf {
        let dir = std::path::Path::new(self.path)
            .parent()
            .expect("fixture has a parent testdata dir")
            .join(kind);
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|e| panic!("create {} dir {}: {e}", kind, dir.display()));
        dir
    }
}
