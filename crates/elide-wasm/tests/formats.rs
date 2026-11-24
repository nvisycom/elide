//! Each document format the browser build decodes, redacted end to end
//! through the bindings: the email it holds is found, and the redacted
//! document, decoded again, holds none. An office document's embedded image
//! reaches the image stage, and its properties are stripped.

#![cfg(target_arch = "wasm32")]

mod common;

use elide_wasm::analyzer::Analyzer;
use elide_wasm::anonymizer::Anonymizer;
use elide_wasm::backend::Backend;
use elide_wasm::enricher::Enricher;
use elide_wasm::operator::Operator;
use elide_wasm::orchestrator::Orchestrator;
use elide_wasm::rule::Rule;
use serde_json::Value;
use wasm_bindgen_test::wasm_bindgen_test;

use self::common::{function, ok, patterns, run, run_stripping};

const DOCX: &[u8] = include_bytes!("../../elide/tests/testdata/docx/body_header_hyperlink.docx");
const PPTX: &[u8] = include_bytes!("../../elide/tests/testdata/pptx/slide_text.pptx");
const XLSX: &[u8] = include_bytes!("../../elide/tests/testdata/xlsx/shared_string_cells.xlsx");
const PDF: &[u8] = include_bytes!("../../elide/tests/testdata/pdf/born_digital_text.pdf");
/// A DOCX with an email in its body and a PNG drawn inline.
const DOCX_IMAGE: &[u8] = include_bytes!("../../elide/tests/testdata/docx/embedded_image.docx");

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
    let (redacted, entities) = run(&pipeline(), bytes, hint).await;
    assert!(
        !emails(&entities).is_empty(),
        "no email found: {entities:?}"
    );
    for email in emails(&entities) {
        let value = email["value"].as_str().unwrap_or_default();
        assert!(
            value.contains('@'),
            "an email's value is what it matched: {email}"
        );
    }
    let (_, again) = run(&pipeline(), &redacted, hint).await;
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

/// An image embedded in a DOCX is decoded as a document of its own and
/// reaches the image stage: OCR reads it, and what it reads is found there,
/// in the image's own part beneath the document's.
#[wasm_bindgen_test]
async fn a_docx_image_reaches_the_image_stage() {
    let ocr = function(
        "image",
        "if (image[0] !== 0x89 || image[1] !== 0x50) throw new Error('not a PNG'); \
         return [{ text: 'bob@example.com', x: 0, y: 0, width: 4, height: 4 }];",
    );
    let pipeline = ok(pipeline().with(
        Analyzer::image()
            .enrich(Enricher::ocr(Backend::callback(ocr)))
            .recognize(patterns()),
        Some(Anonymizer::image().rule(Rule::fallback(Operator::cover()))),
    ));
    let (_, entities) = run(&pipeline, DOCX_IMAGE, Some("docx")).await;
    let image = emails(&entities)
        .into_iter()
        .find(|email| email["location"]["kind"] == "image")
        .unwrap_or_else(|| panic!("no email in the image: {entities:?}"));
    assert_eq!(image["value"], "bob@example.com");
    let part: Vec<&str> = image["part"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(part.len(), 2, "{part:?}");
    assert!(part[1].ends_with("image1.png"), "{part:?}");
    assert!(
        emails(&entities)
            .iter()
            .any(|email| email["location"]["kind"] == "text"),
        "the body's email is found too: {entities:?}"
    );
}

/// A DOCX's document properties are stripped under the browser build's
/// default policy and reported, and the redacted document carries none: run
/// again, nothing more is stripped from them.
#[wasm_bindgen_test]
async fn docx_properties_are_stripped() {
    let is_properties = |item: &&Value| item["kind"] == "metadata" && item["item"] == "properties";
    let (redacted, _, stripped) = run_stripping(&pipeline(), DOCX, Some("docx")).await;
    assert!(
        stripped.iter().any(|item| is_properties(&item)),
        "{stripped:?}"
    );
    let (_, _, again) = run_stripping(&pipeline(), &redacted, Some("docx")).await;
    assert!(!again.iter().any(|item| is_properties(&item)), "{again:?}");
}
