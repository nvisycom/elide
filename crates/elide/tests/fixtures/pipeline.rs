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
use elide::redaction::operators::{Erase, Mask, Replace};
use elide::redaction::{Anonymizer, Operator};
use elide::{Analyzer, Result};

/// Outcome of one end-to-end run: the entities that survived dedup and
/// the re-encoded redacted document.
pub struct PipelineOutcome<M: Modality> {
    /// Entities detected and reconciled, in source coordinates.
    pub entities: Vec<Entity<M>>,
    /// Re-encoded document after redaction.
    pub redacted: String,
}

/// Build the detection side: the real built-in pattern recognizer (with
/// context boosting), behind the standard dedup pipeline. Generic over any
/// text-payload modality the patterns serve.
fn build_analyzer<M: TextBacked>() -> Result<Analyzer<M>>
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
fn build_anonymizer<M: TextBacked>() -> Anonymizer<M>
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

/// A text-format fixture the e2e tests load: the inlined source bytes,
/// the extension the codec registry resolves on, and the on-disk path the
/// redacted artifact is written next to.
///
/// Construct one per format with [`include_str!`] for `source` and the
/// matching `testdata/` path, then call [`run`](Self::run).
pub struct Fixture {
    /// Absolute path to the fixture on disk; the artifact writer derives
    /// `{stem}.redacted.{ext}` next to it.
    pub path: &'static str,
    /// Fixture body the codec decodes (compile-time inlined).
    pub source: &'static str,
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

    /// Decode → analyze → anonymize → encode this fixture as modality `M`,
    /// write the `*.redacted.*` artifact, and return the outcome. Panics
    /// with a descriptive message on any stage failure, the way an
    /// integration test wants.
    async fn run_typed<M>(&self) -> PipelineOutcome<M>
    where
        M: TextBacked,
        DocumentHandle<M>: StreamDataReader<M>,
        PatternRecognizer: Recognizer<M>,
        Replace: Operator<M>,
        Mask: Operator<M>,
        Erase: Operator<M>,
    {
        let handle = FormatRegistry::with_builtin()
            .decode(self.source, self.extension)
            .await
            .unwrap_or_else(|e| panic!("{} source decodes: {e}", self.extension));
        let mut document: DocumentHandle<M> = handle
            .into::<M>()
            .unwrap_or_else(|_| panic!("{} resolves to the expected modality", self.extension));

        let analyzer = build_analyzer::<M>().expect("analyzer builds");
        let anonymizer = build_anonymizer::<M>();

        let en = Language::asserted(LanguageTag::parse("en").unwrap());
        let scope = Scope::new().with_language(en);

        let entities = analyzer
            .analyze_stream(&mut document, &scope)
            .await
            .expect("analyze succeeds");
        anonymizer
            .anonymize(&mut document, &entities)
            .await
            .expect("anonymize succeeds");

        let encoded = document.encode().expect("encode succeeds");
        let redacted =
            String::from_utf8(encoded.as_bytes().to_vec()).expect("codec re-encodes to UTF-8");

        self.write_artifact(&redacted);
        PipelineOutcome { entities, redacted }
    }

    /// Write `redacted` next to the fixture as `{stem}.redacted.{ext}` for
    /// inspection. Gitignored via `**/testdata/**/*.redacted.*`.
    fn write_artifact(&self, redacted: &str) {
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
