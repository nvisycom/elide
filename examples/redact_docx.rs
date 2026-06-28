//! End-to-end example: detect and redact PII in a DOCX *container*.
//!
//! Where the plain-text example ([`redact_txt`]) drives a single text
//! body, a `.docx` is a *container of parts across modalities*: body text
//! in `word/document.xml` plus embedded images in `word/media/*`. This
//! example wires the [`Orchestrator`], which drives the body and every
//! container part through the right per-modality pipeline:
//!
//! 1. [`FormatRegistry`] decodes the `.docx` into an
//!    [`UntypedDocumentHandle`].
//! 2. An [`Orchestrator`] is assembled with a text pipeline for the body
//!    and an image pipeline for the embedded media (mock LLM backend, so
//!    the example runs offline — swap in a real backend to detect in
//!    images).
//! 3. [`analyze`] detects across the body *and* each part, returning an
//!    editable [`Report`] that keeps the body's entities and each part's
//!    entities separated. We print what was found, grouped by part, then
//!    [`anonymize_with`] redacts everything and re-zips the package.
//! 4. We write the redacted `.docx` back out — a complete, drop-in
//!    replacement package, only the redacted parts changed.
//!
//! Run with: `cargo run -p elide-examples --bin redact_docx`.
//!
//! [`redact_txt`]: ./redact_txt.rs
//! [`UntypedDocumentHandle`]: elide::codec::UntypedDocumentHandle
//! [`Orchestrator`]: elide::Orchestrator
//! [`Report`]: elide::Report
//! [`analyze`]: elide::Orchestrator::analyze
//! [`anonymize_with`]: elide::Orchestrator::anonymize_with

use elide::prelude::operators::*;
use elide::prelude::*;
use elide::recognition::llm::LlmRecognizer;
use elide::recognition::ner::NerRecognizer;
use elide::recognition::pattern::PatternRecognizer;

/// A real `.docx` package baked into the binary so the example is
/// self-contained: body text plus one embedded image.
const SAMPLE: &[u8] = include_bytes!("data/contact.docx");

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Decode the container through the codec layer. The orchestrator
    //    works on the untyped handle — it discovers the body's modality by
    //    trial, so no `.into::<Text>()` turbofish is needed here.
    let registry = FormatRegistry::with_builtin();
    let mut document = registry.decode(SAMPLE, "docx").await?;

    // 2. Assemble the orchestrator: one shared scope, plus a text pipeline
    //    for the body and an image pipeline for the embedded media. The
    //    scope is modality-free, so it is set once for every pipeline.
    let en = Language::asserted(LanguageTag::parse("en").unwrap());
    let orchestrator = Orchestrator::new(&registry)
        .with_scope(Scope::new().with_language(en))
        .with_modality::<Text>(build_text_analyzer()?, build_text_anonymizer())
        .with_modality::<Image>(build_image_analyzer()?, build_image_anonymizer());

    // 3. Detect across the body and every container part. The report keeps
    //    each part's findings separate so you can inspect (and edit) them
    //    before anything is redacted.
    let mut report = orchestrator.analyze(&mut document).await?;
    print_report(&mut report);

    // 4. Apply the (here unedited) report: redact the body, redact each
    //    part, write the parts back, and re-encode the package.
    orchestrator.anonymize_with(&mut document, report).await?;
    let encoded = document.encode()?;

    // 5. Write the redacted `.docx` out. That is the whole deliverable: a
    //    drop-in replacement package the caller saves or forwards.
    let out_path = concat!(env!("CARGO_MANIFEST_DIR"), "/data/contact.redacted.docx");
    std::fs::write(out_path, encoded.as_bytes()).expect("write redacted docx");
    println!(
        "\nwrote redacted {} bytes -> {out_path}",
        encoded.as_bytes().len()
    );

    Ok(())
}

/// Print the detected entities grouped by where they live: the body, then
/// each container part by its id. This is the detect → *inspect* → apply
/// seam the orchestrator exposes via the [`Report`].
fn print_report(report: &mut Report) {
    let body = report.entities::<Text>().map(|v| v.len()).unwrap_or(0);
    println!("--- detected ---");
    println!("body: {body} entities");

    // `part_ids` hands back each part's id and modality; the typed
    // accessor reads its entities back.
    let part_ids: Vec<_> = report.part_ids().map(|(id, _)| id.clone()).collect();
    if part_ids.is_empty() {
        println!("parts: none with a matching pipeline");
    }
    for id in part_ids {
        let n = report
            .part_entities::<Image>(&id)
            .map(|v| v.len())
            .unwrap_or(0);
        println!("part {id}: {n} entities");
    }
}

/// Build the body-text analyzer: the real built-in pattern recognizer
/// (with context boosting) plus mock NER and LLM recognizers, behind the
/// standard dedup pipeline.
fn build_text_analyzer() -> Result<Analyzer<Text>> {
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

/// Build the body anonymizer: an operator per label, plus a fallback.
fn build_text_anonymizer() -> Anonymizer<Text> {
    Anonymizer::new()
        .with_label(builtins::EMAIL_ADDRESS.to_ref(), Replace::new("[EMAIL]"))
        .with_label(builtins::PHONE_NUMBER.to_ref(), Replace::new("[PHONE]"))
        .with_label(builtins::IBAN.to_ref(), Replace::new("[IBAN]"))
        .with_label(
            builtins::GOVERNMENT_ID.to_ref(),
            Replace::new("[GOVERNMENT_ID]"),
        )
        .with_label(builtins::IP_ADDRESS.to_ref(), Replace::new("[IP]"))
        // Keep the last four digits of a card visible, mask the rest.
        .with_label(
            builtins::PAYMENT_CARD.to_ref(),
            Mask::stars().with_keep_suffix(4),
        )
        .with_fallback(Erase)
}

/// Build the image analyzer backed by the mock LLM. It detects nothing
/// offline, but proves the multi-modal container path runs end to end;
/// swap in a real backend to redact inside embedded images.
fn build_image_analyzer() -> Result<Analyzer<Image>> {
    let recognizer = LlmRecognizer::<Image>::builder()
        .with_name("image-mock")
        .with_mock_backend()
        .with_default_prompt()
        .build()?;
    Ok(Analyzer::new().with_recognizer(recognizer))
}

/// Build the image anonymizer: blur a detected face, black out a signature,
/// and clear anything else visual. Inert offline (the mock backend detects
/// nothing), but it wires the image redaction operators the real backend
/// would drive.
fn build_image_anonymizer() -> Anonymizer<Image> {
    Anonymizer::new()
        .with_label(builtins::FACE.to_ref(), Blur::new(12.0))
        .with_label(builtins::SIGNATURE.to_ref(), Blackbox::default())
        .with_fallback(Erase)
}
