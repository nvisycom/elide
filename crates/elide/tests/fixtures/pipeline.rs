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
use elide::detection::fuse::{FuseLayer, MaxConfidence};
use elide::detection::resolve::{HighestConfidence, ResolveLayer};
use elide::entity::{Entity, builtins};
#[cfg(feature = "stt")]
use elide::modality::audio::Audio;
#[cfg(any(feature = "llm", feature = "ocr"))]
use elide::modality::image::Image;
#[cfg(feature = "codec-csv")]
use elide::modality::tabular::Tabular;
use elide::modality::text::Text;
use elide::modality::{Modality, StreamDataReader, TextRecognizable};
use elide::primitive::{ConfidenceThreshold, Language, LanguageTag};
use elide::recognition::pattern::PatternRecognizer;
use elide::recognition::{Recognizer, Scope};
use elide::redaction::operators::{Erase, Mask, Replace};
use elide::redaction::{Anonymizer, Operator};
use elide::{EntityGroup, Error, ErrorKind, Orchestrator, Report, Result};

/// Outcome of one end-to-end run: the entities that survived dedup and
/// the re-encoded redacted document.
pub struct PipelineOutcome<M: Modality> {
    /// Entities detected and reconciled, in source coordinates.
    pub entities: Vec<Entity<M>>,
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

    /// Read one entry out of the redacted output, treating it as a zip
    /// container (DOCX). Returns the entry bytes, or `None` if absent.
    pub fn part(&self, name: &str) -> Option<Vec<u8>> {
        use std::io::Read;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(self.redacted.clone())).ok()?;
        let mut entry = zip.by_name(name).ok()?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).ok()?;
        Some(buf)
    }
}

/// Build the detection side: the real built-in pattern recognizer (with
/// context boosting), behind the standard dedup pipeline. Generic over any
/// text-payload modality the patterns serve.
pub fn build_analyzer<M: TextRecognizable>() -> Result<Analyzer<M>> {
    let patterns = PatternRecognizer::builder()
        .with_builtin_patterns()
        .with_builtin_dictionaries()
        .build_context_enhanced()?;

    Ok(Analyzer::new()
        .with_recognizer(patterns)
        .with_layer(FuseLayer::new(MaxConfidence))
        .with_layer(ResolveLayer::new(HighestConfidence))
        .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE)))
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
        .with_label(
            builtins::EMAIL_ADDRESS.to_ref(),
            Replace::new("[email_address]"),
        )
        .with_label(
            builtins::PHONE_NUMBER.to_ref(),
            Replace::new("[phone_number]"),
        )
        .with_label(builtins::IBAN.to_ref(), Replace::new("[iban]"))
        .with_label(
            builtins::GOVERNMENT_ID.to_ref(),
            Replace::new("[government_id]"),
        )
        .with_label(builtins::IP_ADDRESS.to_ref(), Replace::new("[ip_address]"))
        .with_label(builtins::PAYMENT_CARD.to_ref(), Mask::stars())
        .with_fallback(Erase)
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
    /// `{stem}.out.{ext}` next to it.
    pub path: &'static str,
    /// Fixture body the codec decodes (compile-time inlined bytes).
    pub source: &'static [u8],
    /// Extension hint the codec registry resolves on (`"txt"`, …).
    pub extension: &'static str,
}

impl Fixture {
    /// Run the pipeline as the [`Text`] modality (`txt`, `json`, `html`,
    /// and a DOCX's body).
    pub async fn run(&self) -> Result<PipelineOutcome<Text>> {
        self.run_typed::<Text>().await
    }

    /// Run the pipeline as the [`Tabular`] modality (`csv`).
    ///
    /// [`Tabular`]: elide::modality::tabular::Tabular
    #[cfg(feature = "codec-csv")]
    pub async fn run_tabular(&self) -> Result<PipelineOutcome<Tabular>> {
        self.run_typed::<Tabular>().await
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
        let mut document =
            UntypedDocumentHandle::new(self.decode_as::<Audio>(&registry).await?);

        // The mock STT backend transcribes nothing, so recognition finds
        // nothing; the anonymizer would silence/erase any time spans it did.
        let analyzer = Analyzer::new().with_enricher(SttEnricher::new(MockBackend));
        let anonymizer = Anonymizer::new()
            .with_label(builtins::PHONE_NUMBER.to_ref(), Silence)
            .with_fallback(Erase);

        let orchestrator =
            Orchestrator::new(&registry).with_modality::<Audio>(analyzer, anonymizer, Scope::new());

        let mut report = orchestrator.analyze(&mut document).await?;
        let entities: Vec<Entity<Audio>> = report
            .entities::<Audio>()
            .map(|e| e.to_vec())
            .unwrap_or_default();
        self.write_entities(&report);
        orchestrator.anonymize_with(&mut document, report).await?;

        let redacted = document.encode()?.as_bytes().to_vec();
        self.write_redacted(&redacted);
        Ok(PipelineOutcome { entities, redacted })
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
        let mut document =
            UntypedDocumentHandle::new(self.decode_as::<Image>(&registry).await?);

        // The mock OCR backend recognizes nothing, so recognition finds
        // nothing; the anonymizer would clear any regions it did.
        let analyzer = Analyzer::new().with_enricher(OcrEnricher::new(MockBackend));
        let anonymizer = Anonymizer::new().with_fallback(Erase);

        let orchestrator =
            Orchestrator::new(&registry).with_modality::<Image>(analyzer, anonymizer, Scope::new());

        let mut report = orchestrator.analyze(&mut document).await?;
        let entities: Vec<Entity<Image>> = report
            .entities::<Image>()
            .map(|e| e.to_vec())
            .unwrap_or_default();
        self.write_entities(&report);
        orchestrator.anonymize_with(&mut document, report).await?;

        let redacted = document.encode()?.as_bytes().to_vec();
        self.write_redacted(&redacted);
        Ok(PipelineOutcome { entities, redacted })
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
    async fn run_typed<M>(&self) -> Result<PipelineOutcome<M>>
    where
        M: TextRecognizable,
        Entity<M>: Clone,
        Vec<Entity<M>>: EntityGroup,
        DocumentHandle<M>: StreamDataReader<M>,
        Replace: Operator<M>,
        Mask: Operator<M>,
        Erase: Operator<M>,
    {
        let registry = FormatRegistry::with_builtin();
        let mut document = UntypedDocumentHandle::new(self.decode_as::<M>(&registry).await?);

        let en_tag = LanguageTag::parse("en")
            .map_err(|e| Error::new(ErrorKind::Validation, format!("invalid language tag: {e}")))?;
        let scope = Scope::new().with_language(Language::asserted(en_tag));

        let orchestrator = Orchestrator::new(&registry).with_modality::<M>(
            build_analyzer::<M>()?,
            build_anonymizer::<M>(),
            scope,
        );
        // Drive embedded images too when the image recognizer is available.
        #[cfg(feature = "llm")]
        let orchestrator = orchestrator.with_modality::<Image>(
            build_image_analyzer()?,
            Anonymizer::new(),
            Scope::new(),
        );

        // Two phases so the entities surface for assertions: detect, copy
        // the body entities out, then apply with no editing.
        let mut report = orchestrator.analyze(&mut document).await?;
        let entities: Vec<Entity<M>> = report
            .entities::<M>()
            .map(|e| e.to_vec())
            .unwrap_or_default();
        // Write the detected entities as JSON for inspection before the
        // report is consumed by `anonymize_with`.
        self.write_entities(&report);
        orchestrator.anonymize_with(&mut document, report).await?;

        let redacted = document.encode()?.as_bytes().to_vec();

        self.write_redacted(&redacted);
        Ok(PipelineOutcome { entities, redacted })
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
                ErrorKind::Validation,
                format!(
                    "{} did not resolve to the {} modality",
                    self.extension,
                    M::NAME
                ),
            )
        })
    }

    /// Write the serialized detection [`Report`] next to the fixture as
    /// `{filename}.json` (e.g. `sample.docx.json`): the body and any
    /// container parts' findings, grouped by part id. Only with the `serde`
    /// feature; gitignored under `testdata/`.
    #[cfg(feature = "serde")]
    fn write_entities(&self, report: &Report) {
        let out = format!("{}.json", self.path);
        let json = serde_json::to_string_pretty(report).expect("report serializes");
        std::fs::write(&out, json).unwrap_or_else(|e| panic!("write entities {out}: {e}"));
    }

    /// No-op when `serde` is off.
    #[cfg(not(feature = "serde"))]
    fn write_entities(&self, _report: &Report) {}

    /// Write the redacted document next to the fixture as
    /// `{stem}.out.{ext}`. Gitignored via `**/testdata/**/*.out.*`.
    fn write_redacted(&self, redacted: &[u8]) {
        let path = std::path::Path::new(self.path);
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("fixture has a UTF-8 stem");
        let parent = path.parent().expect("fixture has a parent");
        let out = parent.join(format!("{stem}.out.{}", self.extension));
        std::fs::write(&out, redacted)
            .unwrap_or_else(|e| panic!("write redacted artifact {}: {e}", out.display()));
    }
}
