//! The entities a run reports: what each matched, where it is in UTF-16 as well
//! as bytes, which detections found it, and how it was redacted.

#![cfg(target_arch = "wasm32")]

mod common;

use elide_wasm::analyzer::Analyzer;
use elide_wasm::anonymizer::Anonymizer;
use elide_wasm::backend::Backend;
use elide_wasm::enricher::Enricher;
use elide_wasm::operator::Operator;
use elide_wasm::orchestrator::Orchestrator;
use elide_wasm::recognizer::Recognizer;
use elide_wasm::rule::Rule;
use serde_json::{Value, json};
use wasm_bindgen_test::wasm_bindgen_test;

use self::common::{finds, function, ok, patterns, run, run_text};

/// A PNG with no text of its own; the OCR callback says what it shows.
const PNG: &[u8] = include_bytes!("../../elide/tests/testdata/png/plain.png");

/// The only entity of `entities`.
fn only(entities: &[Value]) -> &Value {
    assert_eq!(entities.len(), 1, "{entities:?}");
    &entities[0]
}

/// A text entity carries what it matched, its byte span, and the same span in
/// UTF-16 code units, which differ after a character outside ASCII.
#[wasm_bindgen_test]
async fn a_text_entity_carries_its_value_and_utf16_span() {
    let pipeline = ok(Orchestrator::new().with(
        Analyzer::text().recognize(patterns()),
        Some(Anonymizer::text().rule(Rule::fallback(Operator::remove()))),
    ));
    let (_, entities) = run_text(&pipeline, "Zoë 🙂 <jane@example.com>").await;
    let entity = only(&entities);
    assert_eq!(entity["value"], "jane@example.com");
    // `Zoë 🙂 <` is 11 bytes (`ë` two, `🙂` four), and 8 UTF-16 units (`ë`
    // one, `🙂` two).
    assert_eq!(entity["location"]["start"], 11, "{entity}");
    assert_eq!(
        entity["location"]["utf16"],
        json!({ "start": 8, "end": 24 })
    );
    assert!(entity.get("modality").is_none(), "{entity}");
}

/// The trail says which recognizer and model found an entity, and which
/// operator redacted it and how much that leaks.
#[wasm_bindgen_test]
async fn an_entity_names_its_sources_and_redaction() {
    let llm = function(
        "prompt, request",
        "return { model: 'test-model', \
         entities: [{ value: 'Alice', label: 'person_name', confidence: 0.95, context: 'Alice wrote' }] };",
    );
    let pipeline = ok(Orchestrator::new().with(
        Analyzer::text()
            .recognize(Recognizer::ner(finds("Alice", "person_name")))
            .recognize(ok(Recognizer::llm(Backend::callback(llm), None))),
        Some(Anonymizer::text().rule(Rule::fallback(Operator::remove()))),
    ));
    let (_, entities) = run_text(&pipeline, "Alice wrote this.").await;
    let entity = only(&entities);
    let sources = entity["sources"].as_array().unwrap();
    let models: Vec<&str> = sources.iter().filter_map(|s| s["model"].as_str()).collect();
    assert!(
        models.contains(&"test-model") && models.contains(&"js-callback-ner"),
        "{sources:?}"
    );
    assert!(
        sources.iter().any(|s| s["confidence"] == 0.95),
        "the confidence is the decimal it was given: {sources:?}"
    );
    assert_eq!(
        entity["redaction"],
        json!({ "operator": "remove", "leakProfile": "irrecoverable" })
    );
}

/// An entity left as it is has no redaction: `keep` writes nothing.
#[wasm_bindgen_test]
async fn a_kept_entity_has_no_redaction() {
    let pipeline = ok(Orchestrator::new().with(
        Analyzer::text().recognize(patterns()),
        Some(Anonymizer::text().rule(Rule::fallback(Operator::keep()))),
    ));
    let (_, entities) = run_text(&pipeline, "Mail jane@example.com.").await;
    assert!(only(&entities).get("redaction").is_none(), "{entities:?}");
}

/// A cell entity's value is the part of the cell it matched.
#[wasm_bindgen_test]
async fn a_cell_entity_carries_the_part_of_the_cell_it_matched() {
    let pipeline = ok(Orchestrator::new().with(
        Analyzer::tabular().recognize(patterns()),
        Some(Anonymizer::tabular().rule(Rule::fallback(Operator::remove()))),
    ));
    let csv = b"name,contact\nJane,write to jane@example.com\n";
    let (_, entities) = run(&pipeline, csv, Some("csv")).await;
    let entity = only(&entities);
    assert_eq!(entity["value"], "jane@example.com");
    assert_eq!(entity["location"]["span"], json!({ "start": 9, "end": 25 }));
}

/// An image entity found in OCR text carries the words it matched.
#[wasm_bindgen_test]
async fn an_image_entity_carries_the_ocr_text_it_matched() {
    let ocr = function(
        "image",
        "return [{ text: 'jane@example.com', x: 1, y: 1, width: 8, height: 4 }];",
    );
    let pipeline = ok(Orchestrator::new().with(
        Analyzer::image()
            .enrich(Enricher::ocr(Backend::callback(ocr)))
            .recognize(patterns()),
        None,
    ));
    let (_, entities) = run(&pipeline, PNG, Some("png")).await;
    let email = entities
        .iter()
        .find(|entity| entity["label"] == "email_address")
        .unwrap_or_else(|| panic!("{entities:?}"));
    assert_eq!(email["value"], "jane@example.com");
}
