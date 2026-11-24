//! Helpers the wasm tests share: building handles from JS values, and reading
//! a run's report.

#![allow(dead_code)]

use elide_wasm::analyzer::Analyzer;
use elide_wasm::anonymizer::Anonymizer;
use elide_wasm::backend::Backend;
use elide_wasm::error::ElideError;
use elide_wasm::operator::Operator;
use elide_wasm::orchestrator::Orchestrator;
use elide_wasm::recognizer::Recognizer;
use elide_wasm::rule::Rule;
use js_sys::{Function, JSON, Reflect, Uint8Array};
use serde_json::Value;
use tsify::{Ts, Tsify};
use wasm_bindgen::JsValue;

/// The value of `result`, or a panic with its error's message.
pub fn ok<T>(result: Result<T, ElideError>) -> T {
    result.unwrap_or_else(|error| panic!("{:?}: {}", error.kind(), error.message()))
}

/// The error of `result`, or a panic if it is a value.
pub fn err<T>(result: Result<T, ElideError>) -> ElideError {
    match result {
        Ok(_) => panic!("expected an error"),
        Err(error) => error,
    }
}

/// `json` parsed into a typed config handle.
pub fn config<T: Tsify>(json: &str) -> Ts<T> {
    Ts::new_unchecked(JSON::parse(json).unwrap())
}

/// A JS function of `args` (comma-separated names) running `body`.
pub fn function(args: &str, body: &str) -> Function {
    Function::new_with_args(args, body)
}

/// A NER backend that finds `value` in its text, labelled `label`.
pub fn finds(value: &str, label: &str) -> Backend {
    let body = format!(
        "const i = text.indexOf({value:?}); \
         return i < 0 ? [] : [{{ label: {label:?}, start: i, end: i + {}, score: 0.99 }}];",
        value.len()
    );
    Backend::callback(function("text, labels", &body))
}

/// A text pipeline of `analyzer`, removing what it finds.
pub fn removing(analyzer: Analyzer) -> Orchestrator {
    ok(Orchestrator::new().with(
        analyzer,
        Some(Anonymizer::text().rule(Rule::fallback(Operator::remove()))),
    ))
}

/// A recognizer of the built-in patterns.
pub fn patterns() -> Recognizer {
    ok(Recognizer::pattern(config(
        r#"{"builtinPatterns":true,"builtinDictionaries":false}"#,
    )))
}

/// The redacted bytes and the entities of a run of `pipeline` over `bytes`, of
/// the format `hint` names.
pub async fn run(
    pipeline: &Orchestrator,
    bytes: &[u8],
    hint: Option<&str>,
) -> (Vec<u8>, Vec<Value>) {
    let report: JsValue = ok(pipeline
        .redact(bytes.to_vec(), hint.map(str::to_owned))
        .await)
    .into();
    let redacted = Uint8Array::new(&Reflect::get(&report, &"redacted".into()).unwrap()).to_vec();
    let entities =
        serde_wasm_bindgen::from_value(Reflect::get(&report, &"entities".into()).unwrap()).unwrap();
    (redacted, entities)
}

/// The redacted text and the entities of a run of `pipeline` over `text`.
pub async fn run_text(pipeline: &Orchestrator, text: &str) -> (String, Vec<Value>) {
    let (redacted, entities) = run(pipeline, text.as_bytes(), Some("txt")).await;
    (String::from_utf8(redacted).unwrap(), entities)
}

/// The labels of `entities`.
pub fn labels(entities: &[Value]) -> Vec<&str> {
    entities
        .iter()
        .filter_map(|entity| entity["label"].as_str())
        .collect()
}
