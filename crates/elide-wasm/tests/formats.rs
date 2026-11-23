//! Each document format the browser build decodes, redacted end to end
//! through the bindings: the email it holds is found, and the redacted
//! document, decoded again, holds none.

#![cfg(target_arch = "wasm32")]

use elide_wasm::analyzer::Analyzer;
use elide_wasm::anonymizer::Anonymizer;
use elide_wasm::error::ElideError;
use elide_wasm::operator::Operator;
use elide_wasm::orchestrator::Orchestrator;
use elide_wasm::recognizer::Recognizer;
use elide_wasm::rule::Rule;
use js_sys::{JSON, Reflect, Uint8Array};
use serde_json::Value;
use tsify::Ts;
use wasm_bindgen::JsValue;
use wasm_bindgen_test::wasm_bindgen_test;

const DOCX: &[u8] = include_bytes!("../../elide/tests/testdata/docx/body_header_hyperlink.docx");
const PPTX: &[u8] = include_bytes!("../../elide/tests/testdata/pptx/slide_text.pptx");
const XLSX: &[u8] = include_bytes!("../../elide/tests/testdata/xlsx/shared_string_cells.xlsx");
const PDF: &[u8] = include_bytes!("../../elide/tests/testdata/pdf/born_digital_text.pdf");

/// The value of `result`, or a panic with its error's message.
fn ok<T>(result: Result<T, ElideError>) -> T {
    result.unwrap_or_else(|error| panic!("{:?}: {}", error.kind(), error.message()))
}

/// A recognizer of the built-in patterns.
fn patterns() -> Recognizer {
    let config = JSON::parse(r#"{"builtinPatterns":true,"builtinDictionaries":false}"#).unwrap();
    ok(Recognizer::pattern(Ts::new_unchecked(config)))
}

/// An anonymizer removing everything found.
fn removing(anonymizer: Anonymizer) -> Anonymizer {
    anonymizer.rule(Rule::fallback(Operator::remove()))
}

/// A pipeline finding the built-in patterns in text and cells, and removing
/// them.
fn pipeline() -> Orchestrator {
    let text = ok(Orchestrator::new().with(
        Analyzer::text().recognize(patterns()),
        Some(removing(Anonymizer::text())),
    ));
    ok(text.with(
        Analyzer::tabular().recognize(patterns()),
        Some(removing(Anonymizer::tabular())),
    ))
}

/// The redacted bytes and the entities of a run over `bytes`, of the format
/// `hint` names.
async fn redact(bytes: &[u8], hint: Option<&str>) -> (Vec<u8>, Vec<Value>) {
    let report: JsValue = ok(pipeline()
        .redact(bytes.to_vec(), hint.map(str::to_owned))
        .await)
    .into();
    let redacted = Uint8Array::new(&Reflect::get(&report, &"redacted".into()).unwrap()).to_vec();
    let entities =
        serde_wasm_bindgen::from_value(Reflect::get(&report, &"entities".into()).unwrap()).unwrap();
    (redacted, entities)
}

/// The entities of `entities` labelled an email address.
fn emails(entities: &[Value]) -> Vec<&Value> {
    entities
        .iter()
        .filter(|entity| entity["label"] == "email_address")
        .collect()
}

/// `bytes` has an email found and removed: decoding the redacted document
/// again finds none.
async fn removes_emails(bytes: &[u8], hint: Option<&str>) -> Vec<Value> {
    let (redacted, entities) = redact(bytes, hint).await;
    assert!(
        !emails(&entities).is_empty(),
        "no email found: {entities:?}"
    );
    let (_, again) = redact(&redacted, hint).await;
    assert!(emails(&again).is_empty(), "an email is left: {again:?}");
    entities
}

#[wasm_bindgen_test]
async fn docx_emails_are_removed() {
    removes_emails(DOCX, Some("docx")).await;
}

#[wasm_bindgen_test]
async fn pptx_emails_are_removed() {
    removes_emails(PPTX, Some("pptx")).await;
}

#[wasm_bindgen_test]
async fn xlsx_emails_are_removed() {
    removes_emails(XLSX, Some("xlsx")).await;
}

/// A PDF's text is located where it is drawn: its page, and a box on it.
#[wasm_bindgen_test]
async fn pdf_emails_are_removed_where_they_are_drawn() {
    let entities = removes_emails(PDF, Some("pdf")).await;
    let rendered = &emails(&entities)[0]["location"]["rendered"];
    assert_eq!(rendered["page"], 1, "{rendered}");
    assert_eq!(
        rendered["rects"].as_array().map(Vec::len),
        Some(1),
        "{rendered}"
    );
}

/// A blob with no hint is recognized by its leading bytes.
#[wasm_bindgen_test]
async fn a_blob_without_a_hint_is_sniffed() {
    removes_emails(PDF, None).await;
    removes_emails(DOCX, None).await;
}
