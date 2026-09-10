#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

//! WebAssembly bindings that run the Elide detect-and-redact pipeline in a
//! browser.
//!
//! The pipeline is exposed to JavaScript as a set of opaque handles composed by
//! factory functions, mirroring the [`elide`] facade's own split between
//! detection and redaction. JavaScript builds a [`RecognizerHandle`] from a
//! [`PatternRecognizerConfig`], folds one or more recognizers into an
//! [`AnalyzerHandle`], creates an [`AnonymizerHandle`] policy, and runs them
//! over a piece of text with [`redact`]. The rich Rust objects stay in wasm
//! memory behind the handles; only the config and the [`RedactionResult`] cross
//! the boundary as data.
//!
//! The whole pipeline is `async`, and on wasm its futures are driven by the
//! browser's own event loop through [`wasm_bindgen_futures`] — there is no Tokio
//! runtime.

mod analyzer;
mod anonymizer;

use elide::prelude::*;
use serde::{Deserialize, Serialize};
use tsify::{Ts, Tsify};
use wasm_bindgen::prelude::*;

pub use self::analyzer::{
    AnalyzerHandle, RecognizerHandle, create_analyzer, create_pattern_recognizer,
};
pub use self::anonymizer::{AnonymizerHandle, create_anonymizer};

/// Install the panic hook once, so a Rust panic surfaces in the browser console
/// with a readable message instead of an opaque `unreachable`.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// Which built-in recognizer sources a [`RecognizerHandle`] draws on.
///
/// `Tsify` generates the matching TypeScript `interface` (with camelCase
/// fields), so the JS side passes a typed `{ builtinPatterns, builtinDictionaries }`
/// object. With both `false`, the recognizer detects nothing. It crosses the
/// boundary as a [`Ts<PatternRecognizerConfig>`], tsify's transparent wrapper
/// that deserializes inside the factory without leaking a table slot.
#[derive(Deserialize, Tsify)]
#[serde(rename_all = "camelCase")]
pub struct PatternRecognizerConfig {
    /// Enable the shipped regex patterns (emails, phone numbers, cards, URLs).
    pub builtin_patterns: bool,
    /// Enable the shipped dictionaries.
    pub builtin_dictionaries: bool,
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

/// Detect and redact the personal data in `input` using a composed `analyzer`
/// and `anonymizer`, returning `{ redacted, findings }` as a plain JS object.
///
/// Both handles are borrowed and stay reusable across calls. It is `async`:
/// `await` it from JavaScript.
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
pub async fn redact(
    analyzer: &AnalyzerHandle,
    anonymizer: &AnonymizerHandle,
    input: String,
) -> Result<Ts<RedactionResult>, JsError> {
    let result = run(analyzer, anonymizer, input)
        .await
        .map_err(|e| JsError::new(&e.to_string()))?;
    Ts::from_rust(&result).map_err(|e| JsError::new(&e.to_string()))
}

/// The pipeline proper, kept separate so it returns the crate's own [`Result`]
/// and the wasm boundary only deals with `JsValue`.
async fn run(
    analyzer: &AnalyzerHandle,
    anonymizer: &AnonymizerHandle,
    input: String,
) -> Result<RedactionResult> {
    // Decode the raw text through the codec layer, as any other input would be.
    let registry = FormatRegistry::with_builtin();
    let handle = registry.decode(input, "txt").await?;
    let mut document = handle
        .into::<Text>()
        .expect("the txt codec yields a text document");

    // Detect over the built-in catalog. No language is asserted, so detection
    // is language-agnostic.
    let scope = Scope::new().with_catalog(LabelCatalog::with_builtins());
    let analysis = analyzer
        .analyzer()
        .analyze_stream(&mut document, &scope)
        .await?;
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
        .anonymizer()
        .anonymize(&mut document, &mut entities, &scope)
        .await?;
    let encoded = document.encode()?;
    let redacted = String::from_utf8_lossy(encoded.as_bytes()).into_owned();

    Ok(RedactionResult { redacted, findings })
}
