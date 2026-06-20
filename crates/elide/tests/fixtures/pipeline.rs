//! The shared codec round-trip driver for the e2e tests.
//!
//! Wires the same flow the `redact-txt` example does — decode (codec) →
//! analyze (`Analyzer`) → anonymize (`Anonymizer`) → encode — into one
//! helper the per-format tests call, plus a [`PipelineOutcome`] carrying
//! the entities and re-encoded bytes for assertions.
//!
//! Generic over any [`TextBacked`]: the [`Text`] formats (`txt`,
//! `json`, `html`) and [`Tabular`] (`csv`). The shipped pattern
//! recognizer and the operators serve both — only the codec handle's
//! modality differs.

use elide::codec::{DocumentHandle, FormatRegistry};
use elide::deduplication::filter::FilterLayer;
use elide::deduplication::fuse::{FuseLayer, MaxConfidence};
use elide::deduplication::resolve::{HighestConfidence, ResolveLayer};
use elide::entity::{Entity, builtins};
#[cfg(feature = "codec-csv")]
use elide::modality::tabular::Tabular;
use elide::modality::text::Text;
use elide::modality::{Modality, StreamDataReader, TextBacked};
use elide::primitive::{ConfidenceThreshold, Language, LanguageTag};
use elide::recognition::pattern::PatternRecognizer;
use elide::recognition::{Recognizer, Scope};
use elide::redaction::Operator;
use elide::redaction::operators::{Erase, Mask, Replace};
use elide::{Analyzer, Anonymizer, Orchestrator, Result};

/// Outcome of one end-to-end run: the entities that survived dedup and
/// the re-encoded redacted document.
pub struct PipelineOutcome<M: Modality> {
    /// Entities detected and reconciled, in source coordinates.
    pub entities: Vec<Entity<M>>,
    /// Re-encoded document after redaction, as raw bytes. For text
    /// formats this is UTF-8 — use [`redacted_text`](Self::redacted_text);
    /// for container formats (DOCX) it is the rebuilt package — use
    /// [`part`](Self::part) to read one entry.
    pub redacted: Vec<u8>,
}

impl<M: Modality> PipelineOutcome<M> {
    /// The redacted output decoded as UTF-8 text. Panics if it is not
    /// (i.e. for a binary container format).
    pub fn redacted_text(&self) -> String {
        String::from_utf8(self.redacted.clone()).expect("redacted output is UTF-8 text")
    }
}

/// Build the detection side: the real built-in pattern recognizer (with
/// context boosting), behind the standard dedup pipeline. Generic over any
/// text-payload modality the patterns serve.
pub fn build_analyzer<M: TextBacked>() -> Result<Analyzer<M>>
where
    PatternRecognizer: Recognizer<M>,
{
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
pub fn build_anonymizer<M: TextBacked>() -> Anonymizer<M>
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

/// A codec fixture the e2e tests load: the inlined source bytes, the
/// extension the codec registry resolves on, and the on-disk path the
/// redacted artifact is written next to.
///
/// Construct one per format with [`include_bytes!`] for `source` (so the
/// same shape serves text formats and binary containers like DOCX) and
/// the matching `testdata/` path, then call [`run`](Self::run) /
/// [`run_tabular`](Self::run_tabular).
pub struct Fixture {
    /// Absolute path to the fixture on disk; the artifact writer derives
    /// `{stem}.redacted.{ext}` next to it.
    pub path: &'static str,
    /// Fixture body the codec decodes (compile-time inlined bytes).
    pub source: &'static [u8],
    /// Extension hint the codec registry resolves on (`"txt"`, …).
    pub extension: &'static str,
}

impl Fixture {
    /// Run the pipeline as the [`Text`] modality (`txt`, `json`, `html`).
    pub async fn run(&self) -> PipelineOutcome<Text> {
        self.run_typed::<Text>().await
    }

    /// Run the pipeline as the [`Tabular`](elide::modality::tabular::Tabular)
    /// modality (`csv`).
    #[cfg(feature = "codec-csv")]
    pub async fn run_tabular(&self) -> PipelineOutcome<Tabular> {
        self.run_typed::<Tabular>().await
    }

    /// Decode this fixture as modality `M`, redact it through the
    /// [`Orchestrator`] (body + any container parts), encode, write the
    /// `*.redacted.*` artifact, and return the outcome. Panics with a
    /// descriptive message on any stage failure, the way an integration
    /// test wants.
    ///
    /// One pipeline is registered for `M`; the orchestrator drives the
    /// document's body through it and, for a container fixture (DOCX), any
    /// embedded parts of a registered modality. Per-format media pipelines
    /// are added by the dedicated container tests, not this shared driver.
    async fn run_typed<M>(&self) -> PipelineOutcome<M>
    where
        M: TextBacked,
        Entity<M>: Clone,
        DocumentHandle<M>: StreamDataReader<M>,
        PatternRecognizer: Recognizer<M>,
        Replace: Operator<M>,
        Mask: Operator<M>,
        Erase: Operator<M>,
    {
        let registry = FormatRegistry::with_builtin();
        let mut document: DocumentHandle<M> = registry
            .decode(self.source, self.extension)
            .await
            .unwrap_or_else(|e| panic!("{} source decodes: {e}", self.extension))
            .into::<M>()
            .unwrap_or_else(|_| panic!("{} resolves to the expected modality", self.extension));

        let en = Language::asserted(LanguageTag::parse("en").unwrap());
        let scope = Scope::new().with_language(en);

        let orchestrator = Orchestrator::new(&registry).with_modality::<M>(
            build_analyzer::<M>().expect("analyzer builds"),
            build_anonymizer::<M>(),
            scope,
        );

        // Two phases so the entities surface for assertions: detect, copy
        // the body entities out, then apply with no editing.
        let mut plan = orchestrator
            .analyze_document(&mut document)
            .await
            .expect("analyze document");
        let entities: Vec<Entity<M>> = plan.entities::<M>().map(|e| e.to_vec()).unwrap_or_default();
        orchestrator
            .apply(&mut document, plan)
            .await
            .expect("apply document");

        let redacted = document
            .encode()
            .expect("encode succeeds")
            .as_bytes()
            .to_vec();

        self.write_artifact(&redacted);
        PipelineOutcome { entities, redacted }
    }

    /// Write `redacted` next to the fixture as `{stem}.redacted.{ext}` for
    /// inspection. Gitignored via `**/testdata/**/*.redacted.*`.
    fn write_artifact(&self, redacted: &[u8]) {
        let path = std::path::Path::new(self.path);
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("fixture has a UTF-8 stem");
        let parent = path.parent().expect("fixture has a parent");
        let out = parent.join(format!("{stem}.redacted.{}", self.extension));
        std::fs::write(&out, redacted)
            .unwrap_or_else(|e| panic!("write redacted artifact {}: {e}", out.display()));
    }
}
