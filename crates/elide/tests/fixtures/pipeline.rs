//! The shared codec round-trip driver for the text-format e2e tests.
//!
//! Wires the same flow the `redact-txt` example does — decode (codec) →
//! analyze (`Analyzer`) → anonymize (`Anonymizer`) → encode — into one
//! helper the per-format tests call, plus a [`PipelineOutcome`] carrying
//! the entities and re-encoded bytes for assertions.
//!
//! Scope: the [`Text`] modality only (`txt`, `json`, `html`). The
//! tabular path (CSV) is deferred until the pattern recognizer and the
//! anonymizer operators gain `Tabular` impls — today both are `Text`-only.

use elide::codec::{DocumentHandle, FormatRegistry};
use elide::deduplication::filter::FilterLayer;
use elide::deduplication::fuse::{FuseLayer, MaxConfidence};
use elide::deduplication::resolve::{HighestConfidence, ResolveLayer};
use elide::entity::{Entity, builtins};
use elide::modality::text::Text;
use elide::primitive::{ConfidenceThreshold, Language, LanguageTag};
use elide::recognition::Scope;
use elide::recognition::ner::NerRecognizer;
use elide::recognition::pattern::PatternRecognizer;
use elide::redaction::Anonymizer;
use elide::redaction::operators::{Mask, Redact, Replace};
use elide::{Analyzer, Result};

/// Outcome of one end-to-end run: the entities that survived dedup and
/// the re-encoded redacted document.
pub struct PipelineOutcome {
    /// Entities detected and reconciled, in source coordinates.
    pub entities: Vec<Entity<Text>>,
    /// Re-encoded document after redaction.
    pub redacted: String,
}

/// Build the detection side: the real built-in pattern recognizer (with
/// context boosting) plus a mock NER, behind the standard dedup pipeline.
fn build_analyzer() -> Result<Analyzer<Text>> {
    let patterns = PatternRecognizer::builder()
        .with_builtin_patterns()
        .with_builtin_dictionaries()
        .build_context_enhanced()?;

    // Mock NER: wired like a real model, returns nothing offline. Present
    // so the pipeline shape matches production (multi-recognizer fan-in).
    let ner = NerRecognizer::builder()
        .with_name("ner-mock")
        .with_mock_backend()
        .with_supported_labels(vec![builtins::PERSON_NAME.to_ref()])
        .build()?;

    Ok(Analyzer::new()
        .with_recognizer(patterns)
        .with_recognizer(ner)
        .with_layer(FuseLayer::new(MaxConfidence))
        .with_layer(ResolveLayer::new(HighestConfidence))
        .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE)))
}

/// Build the redaction side: one operator per label the shipped patterns
/// emit, so assertions can spot the replacement tokens, plus a fallback.
fn build_anonymizer() -> Anonymizer<Text> {
    Anonymizer::new()
        .with_operator(
            builtins::EMAIL_ADDRESS.to_ref(),
            Replace::new("[email_address]"),
        )
        .with_operator(
            builtins::PHONE_NUMBER.to_ref(),
            Replace::new("[phone_number]"),
        )
        .with_operator(builtins::IBAN.to_ref(), Replace::new("[iban]"))
        .with_operator(
            builtins::GOVERNMENT_ID.to_ref(),
            Replace::new("[government_id]"),
        )
        .with_operator(builtins::IP_ADDRESS.to_ref(), Replace::new("[ip_address]"))
        .with_operator(builtins::PAYMENT_CARD.to_ref(), Mask::stars())
        .with_fallback(Redact)
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
    /// Decode → analyze → anonymize → encode this fixture, write the
    /// `*.redacted.*` artifact, and return the outcome. Panics with a
    /// descriptive message on any stage failure, the way an integration
    /// test wants.
    pub async fn run(&self) -> PipelineOutcome {
        let handle = FormatRegistry::with_builtin()
            .decode(self.source, self.extension)
            .await
            .unwrap_or_else(|e| panic!("{} source decodes: {e}", self.extension));
        let mut document: DocumentHandle<Text> = handle
            .into::<Text>()
            .expect("text-shaped format yields a text handle");

        let analyzer = build_analyzer().expect("analyzer builds");
        let anonymizer = build_anonymizer();

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
            String::from_utf8(encoded.as_bytes().to_vec()).expect("text codec re-encodes to UTF-8");

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
