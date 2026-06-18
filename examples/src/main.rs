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
//! Run with: `cargo run -p elide-examples --bin redact-txt`.
//!
//! [`DocumentHandle<Text>`]: elide::codec::DocumentHandle
//! [`Analyzer::analyze_stream`]: elide::Analyzer::analyze_stream
//! [`Anonymizer::anonymize`]: elide::Anonymizer::anonymize
//! [`PatternRecognizer`]: elide::pattern::PatternRecognizer
//! [`NerRecognizer`]: elide::ner::NerRecognizer
//! [`LlmRecognizer`]: elide::llm::LlmRecognizer

mod analyzer;
mod anonymizer;

use elide::codec::FormatRegistry;
use elide::modality::text::Text;
use elide::primitive::{Language, LanguageTag, Languages};
use elide::{AnalysisOptions, Result};

/// Sample document baked into the binary so the example is self-contained.
const SAMPLE: &str = include_str!("../data/sample.txt");

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Decode the text file through the codec layer.
    let registry = FormatRegistry::with_builtin();
    let handle = registry.decode(SAMPLE, "txt").await?;
    let mut document = handle
        .into::<Text>()
        .expect("the txt codec yields a text document");

    // 2. Assemble the analyzer once; it is reused for every chunk.
    let analyzer = analyzer::build_analyzer()?;

    // 3. Assemble the anonymizer: an operator per label, plus a fallback.
    let anonymizer = anonymizer::build_anonymizer();

    // 4. Detect: stream the document and get entities already in the
    //    document's source coordinates (lift is folded in). The options
    //    carry per-call assertions (here, that the document is English).
    let english_tag = LanguageTag::parse("en").expect("`en` is a valid tag");
    let english = Language::asserted(english_tag, None);
    let options = AnalysisOptions::builder()
        .with_languages(Languages::new(vec![english]))
        .build()
        .expect("options have defaults");
    let entities = analyzer.analyze_stream(&mut document, &options).await?;

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

/// Tiny pluralization helper for the summary line.
fn plural(n: usize) -> &'static str {
    if n == 1 { "y" } else { "ies" }
}
