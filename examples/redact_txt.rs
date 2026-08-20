//! End-to-end example: detect and redact PII in a plain-text file.
//!
//! Wires the full toolkit pipeline over a `.txt` document:
//!
//! 1. [`FormatRegistry`] decodes the file into a [`DocumentHandle<Text>`].
//! 2. [`Analyzer::analyze_stream`] streams the document and runs three
//!    recognizers concurrently: a real built-in [`PatternRecognizer`]
//!    (emails, phone numbers, payment cards, URLs, …), an
//!    [`NerRecognizer`], and an [`LlmRecognizer`]. The NER and LLM
//!    recognizers use mock backends so the example runs offline with no
//!    API keys; swap in a real backend to see them contribute. The
//!    returned entities are already in the document's coordinates —
//!    chunking and coordinate lifting happen inside `analyze_stream`.
//! 3. Deduplication layers fuse overlapping detections, resolve
//!    conflicts, and drop low-confidence ones.
//! 4. [`Anonymizer::anonymize`] picks a redaction operator per label,
//!    applies the replacements back into the document, and we re-encode
//!    and print the redacted text.
//!
//! Run with: `cargo run -p elide-examples --bin redact_txt`.
//!
//! [`DocumentHandle<Text>`]: elide::codec::DocumentHandle
//! [`Analyzer::analyze_stream`]: elide::detection::Analyzer::analyze_stream
//! [`Anonymizer::anonymize`]: elide::redaction::Anonymizer::anonymize
//! [`PatternRecognizer`]: elide::recognition::pattern::PatternRecognizer
//! [`NerRecognizer`]: elide::recognition::ner::NerRecognizer
//! [`LlmRecognizer`]: elide::recognition::llm::LlmRecognizer

use elide::prelude::operators::*;
use elide::prelude::*;
use elide::recognition::llm::LlmRecognizer;
use elide::recognition::ner::NerRecognizer;
use elide::recognition::pattern::PatternRecognizer;

/// Sample document baked into the binary so the example is self-contained.
const SAMPLE: &str = include_str!("data/sample.txt");

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Decode the text file through the codec layer.
    let registry = FormatRegistry::with_builtin();
    let handle = registry.decode(SAMPLE, "txt").await?;
    let mut document = handle
        .into::<Text>()
        .expect("the txt codec yields a text document");

    // 2. Assemble the analyzer once; it is reused for every chunk.
    let analyzer = build_analyzer()?;

    // 3. Assemble the anonymizer: an operator per label, plus a fallback.
    let anonymizer = build_anonymizer();

    // 4. Detect: stream the document and get entities already in the
    //    document's source coordinates (lift is folded in). The context
    //    carries per-call assertions (here, that the document is English).
    let en = Language::asserted(LanguageTag::parse("en").unwrap());
    let scope = Scope::new().with_language(en);
    let analysis = analyzer.analyze_stream(&mut document, &scope).await?;
    let mut entities = analysis.entities;

    // 5. Redact: apply each entity's operator back into the document,
    //    then re-encode.
    anonymizer
        .anonymize(&mut document, &mut entities, &scope)
        .await?;
    let encoded = document.encode()?;
    let redacted = String::from_utf8_lossy(encoded.as_bytes());

    let count = entities.len();
    println!("\n--- original ---\n{SAMPLE}");
    println!("--- redacted ({count} entities) ---\n{redacted}");
    // Resource usage recorded per recognizer/enricher during detection.
    #[cfg(feature = "usage")]
    for usage in &analysis.usage {
        println!(
            "usage: {} — {} ms, {} found",
            usage.id,
            usage.duration.as_millis(),
            usage.count.unwrap_or(0)
        );
    }

    Ok(())
}

/// Build the three-recognizer analyzer plus its deduplication pipeline.
fn build_analyzer() -> Result<Analyzer<Text>> {
    // Real built-in patterns + dictionaries, with context boosting.
    let patterns = PatternRecognizer::builder()
        .with_builtin_patterns()
        .with_builtin_dictionaries()
        .build_context_enhanced()?;

    // Mock NER: wired like a real model, returns no entities offline.
    let ner = NerRecognizer::builder()
        .with_name("ner-mock")
        .with_mock_backend()
        .with_supported_labels(vec![
            builtins::PERSON_NAME.to_ref(),
            builtins::ADDRESS.to_ref(),
        ])
        .build()?;

    // Mock LLM: wired like a real model, returns no entities offline.
    let llm = LlmRecognizer::<Text>::builder()
        .with_name("llm-mock")
        .with_mock_backend()
        .with_default_prompt()
        .build()?;

    Ok(Analyzer::new()
        .with_recognizer(patterns)
        .with_recognizer(ner)
        .with_recognizer(llm)
        .with_layer(ReconcileLayer::same_label(Merging::max()))
        .with_layer(ReconcileLayer::cross_label(Structural::default()))
        .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE)))
}

/// Build an anonymizer that picks a redaction strategy per label.
fn build_anonymizer() -> Anonymizer<Text> {
    Anonymizer::new()
        // A weak detection (below the baseline threshold) is kept as-is,
        // before any label rule can fire. Order matters: the first
        // matching rule wins.
        .with(Rule::predicate(
            |cx| !ConfidenceThreshold::BASELINE.passes(cx.entity.confidence),
            Keep,
        ))
        .with(Rule::label(
            builtins::EMAIL_ADDRESS.to_ref(),
            Replace::new("[EMAIL]"),
        ))
        .with(Rule::label(
            builtins::PHONE_NUMBER.to_ref(),
            Replace::new("[PHONE]"),
        ))
        .with(Rule::label(builtins::URL.to_ref(), Replace::new("[URL]")))
        // Keep the last four digits of a card visible, mask the rest.
        .with(Rule::label(
            builtins::PAYMENT_CARD.to_ref(),
            Mask::stars().with_keep_suffix(4),
        ))
        // Anything else we detect gets fully removed.
        .with(Rule::fallback(Erase))
}
