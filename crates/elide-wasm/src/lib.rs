//! WebAssembly bindings that run the Elide detect-and-redact pipeline in a
//! browser.
//!
//! This is a thin boundary over the [`elide`] facade: it decodes a piece of
//! text, runs the built-in pattern recognizers over it, and applies a
//! per-label redaction policy, returning the redacted text plus the list of
//! entities that were found. The whole pipeline is `async`, and on wasm its
//! futures are driven by the browser's own event loop through
//! [`wasm_bindgen_futures`] — there is no Tokio runtime.
//!
//! The exported [`redact_text`] function is the single entry point JavaScript
//! calls; everything else is private wiring.

use elide::prelude::operators::*;
use elide::prelude::*;
use elide::recognition::pattern::PatternRecognizer;
use serde::Serialize;
use tsify::{Ts, Tsify};
use wasm_bindgen::prelude::*;

/// Install the panic hook once, so a Rust panic surfaces in the browser console
/// with a readable message instead of an opaque `unreachable`.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// One detected entity, in the shape the JavaScript side consumes.
///
/// `Tsify` generates the matching TypeScript `interface`, so the JS side sees a
/// typed `Finding` rather than an opaque object.
#[derive(Serialize, Tsify)]
pub struct Finding {
    /// The entity's label id (e.g. `email_address`).
    pub label: String,
    /// Start byte offset of the match in the original text.
    pub start: usize,
    /// End byte offset (exclusive) of the match in the original text.
    pub end: usize,
    /// Detection confidence in `[0, 1]`.
    pub confidence: f32,
}

/// The result handed back to JavaScript: the redacted text and what was found.
#[derive(Serialize, Tsify)]
pub struct RedactionResult {
    /// The input text with every matched entity replaced by its policy output.
    pub redacted: String,
    /// Every entity the pipeline detected, in document order.
    pub findings: Vec<Finding>,
}

/// Detect and redact the personal data in `input`, returning
/// `{ redacted, findings }` as a plain JS object.
///
/// This runs the built-in pattern + dictionary recognizers (emails, phone
/// numbers, payment cards, URLs, and the shipped dictionaries) and applies a
/// per-label redaction policy. It is `async`: `await` it from JavaScript.
///
/// # Errors
///
/// Rejects with a JS error string if the pipeline fails (e.g. the text cannot
/// be decoded or analyzed).
///
/// Returns a [`Ts<RedactionResult>`], tsify's transparent wrapper that carries a
/// typed value across the wasm boundary without leaking: deserialization happens
/// here, inside the function, so destructors run normally. The generated
/// TypeScript still reports the return as `Promise<RedactionResult>`.
#[wasm_bindgen]
pub async fn redact_text(input: String) -> Result<Ts<RedactionResult>, JsError> {
    let result = run(input).await.map_err(|e| JsError::new(&e.to_string()))?;
    Ts::from_rust(&result).map_err(|e| JsError::new(&e.to_string()))
}

/// The pipeline proper, kept separate so it returns the crate's own [`Result`]
/// and the wasm boundary only deals with `JsValue`.
async fn run(input: String) -> Result<RedactionResult> {
    // Decode the raw text through the codec layer, as any other input would be.
    let registry = FormatRegistry::with_builtin();
    let handle = registry.decode(input, "txt").await?;
    let mut document = handle
        .into::<Text>()
        .expect("the txt codec yields a text document");

    let analyzer = build_analyzer()?;
    let anonymizer = build_anonymizer();

    // Detect over the built-in catalog. No language is asserted, so detection
    // is language-agnostic.
    let scope = Scope::new().with_catalog(LabelCatalog::with_builtins());
    let analysis = analyzer.analyze_stream(&mut document, &scope).await?;
    let mut entities = analysis.entities;

    let findings = entities
        .iter()
        .filter_map(|entity| {
            let range = entity.location.range()?;
            Some(Finding {
                label: entity.label.as_str().to_owned(),
                start: range.start,
                end: range.end,
                confidence: f32::from(entity.confidence),
            })
        })
        .collect();

    // Apply the redaction policy and re-encode the document back to text.
    anonymizer
        .anonymize(&mut document, &mut entities, &scope)
        .await?;
    let encoded = document.encode()?;
    let redacted = String::from_utf8_lossy(encoded.as_bytes()).into_owned();

    Ok(RedactionResult { redacted, findings })
}

/// The pattern-recognizer analyzer plus its deduplication pipeline. Mirrors the
/// native `redact_txt` example, minus the offline NER/LLM mocks (which add
/// nothing in a browser and only enlarge the build).
fn build_analyzer() -> Result<Analyzer<Text>> {
    let patterns = PatternRecognizer::builder()
        .with_builtin_patterns()
        .with_builtin_dictionaries()
        .build_context_enhanced()?;

    Ok(Analyzer::new()
        .with_recognizer(patterns)
        .with_layer(ReconcileLayer::same_label(Merging::max()))
        .with_layer(ReconcileLayer::cross_label(Structural::default()))
        .with_layer(FilterLayer::new().with_threshold(ConfidenceThreshold::BASELINE)))
}

/// A redaction policy: a readable token per common label, a masked tail for
/// payment cards, and full erasure for anything else detected.
fn build_anonymizer() -> Anonymizer<Text> {
    Anonymizer::new()
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
        .with(Rule::label(
            builtins::PAYMENT_CARD.to_ref(),
            Mask::stars().with_keep_suffix(4),
        ))
        .with(Rule::fallback(Erase))
}
