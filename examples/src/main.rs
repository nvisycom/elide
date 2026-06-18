//! End-to-end example: detect and redact PII in a plain-text file.
//!
//! Wires the full toolkit pipeline over a `.txt` document:
//!
//! 1. [`CodecRegistry`] decodes the file into a [`DocumentHandle<Text>`].
//! 2. [`Analyzer::analyze_stream`] streams the document and runs three
//!    recognizers concurrently: a real built-in [`PatternRecognizer`]
//!    (emails, phone numbers, payment cards, URLs, …), an
//!    [`NerRecognizer`], and an [`LlmRecognizer`]. The NER and LLM
//!    recognizers use no-op backends so the example runs offline with no
//!    API keys; swap in a real backend to see them contribute. The
//!    returned entities are already in the document's coordinates —
//!    chunking and coordinate lifting happen inside `analyze_stream`.
//! 3. Deduplication layers fuse overlapping detections, resolve
//!    conflicts, and drop low-confidence ones.
//! 4. [`Anonymizer::anonymize`] picks a redaction operator per label,
//!    applies the replacements back into the document, and we re-encode
//!    and print the redacted text.
//!
//! Run with: `cargo run -p veil-examples --bin redact-txt`.
//!
//! [`DocumentHandle<Text>`]: veil_codec::DocumentHandle
//! [`Analyzer::analyze_stream`]: veil_toolkit::Analyzer::analyze_stream
//! [`Anonymizer::anonymize`]: veil_toolkit::Anonymizer::anonymize
//! [`PatternRecognizer`]: veil_pattern::PatternRecognizer
//! [`NerRecognizer`]: veil_ner::NerRecognizer
//! [`LlmRecognizer`]: veil_llm::LlmRecognizer

use veil_codec::CodecRegistry;
use veil_core::entity::builtins;
use veil_core::modality::text::Text;
use veil_core::primitive::ConfidenceThreshold;
use veil_core::Result;

use veil_llm::backend::NoopBackend as LlmNoopBackend;
use veil_llm::{DefaultPrompt, LlmRecognizer};
use veil_ner::backend::NoopBackend as NerNoopBackend;
use veil_ner::NerRecognizer;
use veil_pattern::PatternRecognizer;

use veil_toolkit::deduplication::filter::FilterLayer;
use veil_toolkit::deduplication::fuse::{FuseLayer, MaxConfidence};
use veil_toolkit::deduplication::resolve::{HighestConfidence, ResolveLayer};
use veil_toolkit::operators::{Mask, Redact, Replace};
use veil_toolkit::{Analyzer, Anonymizer};

/// Sample document baked into the binary so the example is self-contained.
const SAMPLE: &str = include_str!("../data/sample.txt");

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Decode the text file through the codec layer.
    let registry = CodecRegistry::with_builtin();
    let handle = registry.decode(SAMPLE, "txt").await?;
    let mut document = handle
        .into::<Text>()
        .expect("the txt codec yields a text document");

    // 2. Assemble the analyzer once; it is reused for every chunk.
    let analyzer = build_analyzer()?;

    // 3. Assemble the anonymizer: an operator per label, plus a fallback.
    let anonymizer = build_anonymizer();

    // 4. Detect: stream the document and get entities already in the
    //    document's source coordinates (lift is folded in).
    let entities = analyzer.analyze_stream(&mut document).await?;

    // 5. Redact: apply each entity's operator back into the document,
    //    then re-encode.
    anonymizer.anonymize(&mut document, &entities).await?;
    let encoded = document.encode()?;
    let redacted = String::from_utf8_lossy(encoded.as_bytes());

    let count = entities.len();
    println!("--- detected {count} entit{} ---", plural(count));
    println!("\n--- original ---\n{SAMPLE}");
    println!("--- redacted ---\n{redacted}");

    Ok(())
}

/// Build the three-recognizer analyzer plus its deduplication pipeline.
fn build_analyzer() -> Result<Analyzer<Text>> {
    // Real built-in patterns + dictionaries, with context boosting.
    let patterns = PatternRecognizer::builder()
        .with_builtin_patterns()
        .with_builtin_dictionaries()
        .build_context_enhanced()?;

    // No-op NER: wired like a real model, returns no spans offline.
    let ner = NerRecognizer::builder()
        .with_name("ner-noop")
        .with_backend(NerNoopBackend)
        .with_supported_labels(vec![
            builtins::PERSON_NAME.to_ref(),
            builtins::ADDRESS.to_ref(),
        ])
        .build()?;

    // No-op LLM: wired like a real provider, returns no entities offline.
    let llm = LlmRecognizer::<Text>::builder()
        .with_name("llm-noop")
        .with_backend(LlmNoopBackend)
        .with_prompt(DefaultPrompt)
        .build()?;

    Ok(Analyzer::new()
        .with_recognizer(patterns)
        .with_recognizer(ner)
        .with_recognizer(llm)
        .with_layer(FuseLayer::new(MaxConfidence))
        .with_layer(ResolveLayer::new(HighestConfidence))
        .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE)))
}

/// Build an anonymizer that picks a redaction strategy per label.
fn build_anonymizer() -> Anonymizer<Text> {
    Anonymizer::new()
        .with_operator(builtins::EMAIL_ADDRESS.to_ref(), Replace::new("[EMAIL]"))
        .with_operator(builtins::PHONE_NUMBER.to_ref(), Replace::new("[PHONE]"))
        .with_operator(builtins::URL.to_ref(), Replace::new("[URL]"))
        // Keep the last four digits of a card visible, mask the rest.
        .with_operator(
            builtins::PAYMENT_CARD.to_ref(),
            Mask::stars().with_keep_suffix(4),
        )
        // Anything else we detect gets fully removed.
        .with_fallback(Redact)
}

/// Tiny pluralization helper for the summary line.
fn plural(n: usize) -> &'static str {
    if n == 1 { "y" } else { "ies" }
}
