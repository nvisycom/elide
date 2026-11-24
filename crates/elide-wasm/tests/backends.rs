//! The JS-callback backends and the decorators around them, run end to end
//! through the bindings: the LLM recognizer, a fallback for failed calls, the
//! language enricher and the routes that follow its languages and the asserted
//! ones, and the rate and concurrency limits.

#![cfg(target_arch = "wasm32")]

mod common;

use elide_wasm::analyzer::Analyzer;
use elide_wasm::backend::Backend;
use elide_wasm::enricher::Enricher;
use elide_wasm::error::ElideErrorKind;
use elide_wasm::orchestrator::Orchestrator;
use elide_wasm::recognizer::Recognizer;
use js_sys::Date;
use wasm_bindgen_test::wasm_bindgen_test;

use self::common::{config, err, finds, function, labels, ok, removing, run_text};

/// A NER backend that finds `Alice`.
fn finds_alice() -> Backend {
    finds("Alice", "person_name")
}

/// A NER backend that finds nothing.
fn finds_nothing() -> Backend {
    Backend::callback(function("text, labels", "return [];"))
}

/// A PNG with no text of its own.
const PNG: &[u8] = include_bytes!("../../elide/tests/testdata/png/plain.png");

/// A backend whose every call fails with `down`.
fn fails() -> Backend {
    Backend::callback(function("", "throw new Error('down');"))
}

/// A text pipeline of `recognizer`, removing what it finds.
fn removing_found_by(recognizer: Recognizer) -> Orchestrator {
    removing(Analyzer::text().recognize(recognizer))
}

/// The LLM callback gets the prompt holding the text, and the request naming
/// the stage and the answer's schema; the value it answers is found in the text
/// and removed.
#[wasm_bindgen_test]
async fn an_llm_recognizer_finds_what_its_model_answers() {
    let callback = function(
        "prompt, request",
        "if (!prompt.includes('Alice wrote this.')) throw new Error('no text in the prompt'); \
         if (request.modality !== 'text') throw new Error('modality ' + request.modality); \
         if (!request.schema.properties.entities) throw new Error('no schema'); \
         if (request.image !== undefined) throw new Error('an image on a text stage'); \
         return { model: 'test-model', \
                  entities: [{ value: 'Alice', label: 'person_name', confidence: 0.99, context: 'Alice wrote' }] };",
    );
    let recognizer = ok(Recognizer::llm(Backend::callback(callback), None));
    let (redacted, entities) = run_text(&removing_found_by(recognizer), "Alice wrote this.").await;
    assert_eq!(labels(&entities), ["person_name"]);
    assert!(!redacted.contains("Alice"), "{redacted}");
}

/// On an image stage the request carries the image as PNG bytes in a
/// `Uint8Array`, and the region the callback answers is found.
#[wasm_bindgen_test]
async fn an_llm_recognizer_gets_the_image_as_png_bytes() {
    let callback = function(
        "prompt, request",
        "if (request.modality !== 'image') throw new Error('modality ' + request.modality); \
         if (!(request.image instanceof Uint8Array)) throw new Error('image is not a Uint8Array'); \
         if (request.image[0] !== 0x89 || request.image[1] !== 0x50) throw new Error('not a PNG'); \
         return { entities: [{ x_min: 0.1, y_min: 0.1, x_max: 0.5, y_max: 0.5, label: 'person_name', confidence: 0.9 }] };",
    );
    let recognizer = ok(Recognizer::llm(Backend::callback(callback), None));
    let pipeline = ok(Orchestrator::new().with(Analyzer::image().recognize(recognizer), None));
    let (_, entities) = common::run(&pipeline, PNG, Some("png")).await;
    assert_eq!(labels(&entities), ["person_name"]);
}

/// A template replaces the built-in wording.
#[wasm_bindgen_test]
async fn an_llm_template_words_the_prompt() {
    let callback = function(
        "prompt, request",
        "if (prompt !== 'People in: Alice wrote this.') throw new Error(prompt); \
         return { entities: [{ value: 'Alice', label: 'person_name', confidence: 0.99, context: 'Alice wrote' }] };",
    );
    let recognizer = ok(Recognizer::llm(
        Backend::callback(callback),
        Some("People in: {{ text }}".to_owned()),
    ));
    let (redacted, _) = run_text(&removing_found_by(recognizer), "Alice wrote this.").await;
    assert!(!redacted.contains("Alice"), "{redacted}");
}

/// A template that does not compile is refused when the recognizer is built.
#[wasm_bindgen_test]
fn an_llm_template_that_does_not_compile_is_refused() {
    let error = err(Recognizer::llm(finds_nothing(), Some("{{ text".to_owned())));
    assert_eq!(error.kind(), ElideErrorKind::Configuration);
}

/// The LLM recognizer serves text and image stages; a tabular stage refuses it.
#[wasm_bindgen_test]
fn an_llm_recognizer_is_refused_by_a_tabular_stage() {
    let recognizer = ok(Recognizer::llm(finds_nothing(), None));
    let error = err(Orchestrator::new().with(Analyzer::tabular().recognize(recognizer), None));
    assert_eq!(error.kind(), ElideErrorKind::Configuration);
}

/// A call the primary fails goes to the fallback.
#[wasm_bindgen_test]
async fn a_failed_call_goes_to_the_fallback() {
    let backend = fails().fallback_routed(finds_alice());
    let (redacted, _) = run_text(
        &removing_found_by(Recognizer::ner(backend)),
        "Alice wrote this.",
    )
    .await;
    assert!(!redacted.contains("Alice"), "{redacted}");
}

/// Without a fallback, a failed call fails the run with the callback's message.
#[wasm_bindgen_test]
async fn a_failed_call_without_a_fallback_fails_the_run() {
    let pipeline = removing_found_by(Recognizer::ner(fails()));
    let error = err(pipeline
        .redact(b"Alice wrote this.".to_vec(), Some("txt".to_owned()))
        .await);
    assert!(error.message().contains("down"), "{}", error.message());
}

/// A request goes to the route for the asserted language, and a request with
/// none to the backend routed from.
#[wasm_bindgen_test]
async fn requests_go_to_the_backend_for_their_language() {
    let routed = || {
        let backend = ok(finds_nothing()
            .language_routed()
            .route("de".to_owned(), finds_alice()));
        removing_found_by(Recognizer::ner(backend))
    };
    let (redacted, _) = run_text(&routed(), "Alice wrote this.").await;
    assert!(redacted.contains("Alice"), "{redacted}");
    let german = ok(routed().with_languages(vec!["de".to_owned()]));
    let (redacted, _) = run_text(&german, "Alice wrote this.").await;
    assert!(!redacted.contains("Alice"), "{redacted}");
}

/// An invalid asserted language is refused.
#[wasm_bindgen_test]
fn an_invalid_language_is_refused() {
    let error = err(Orchestrator::new().with_languages(vec!["not a tag".to_owned()]));
    assert_eq!(error.kind(), ElideErrorKind::Configuration);
}

/// A NER pipeline whose backend finds `Alice` in German and nothing in any
/// other language, detecting languages with the JS `detect` body over
/// `(text, candidates)`.
fn routed_to_german(detect: &str) -> Orchestrator {
    let backend = ok(finds_nothing()
        .language_routed()
        .route("de".to_owned(), finds_alice()));
    removing(
        Analyzer::text()
            .enrich(ok(Enricher::language(Backend::callback(function(
                "text, candidates",
                detect,
            )))))
            .recognize(Recognizer::ner(backend)),
    )
}

/// A detector that calls a text with `wohnt` German, and anything else
/// English.
const DETECTS: &str =
    "return [{ language: text.includes('wohnt') ? 'de' : 'en', confidence: 0.9 }];";

/// A request goes to the route for the language the JS enricher detects.
#[wasm_bindgen_test]
async fn requests_go_to_the_backend_for_their_detected_language() {
    let pipeline = routed_to_german(DETECTS);
    let (redacted, _) = run_text(&pipeline, "Alice wohnt in Berlin.").await;
    assert!(!redacted.contains("Alice"), "{redacted}");
    let (redacted, _) = run_text(&pipeline, "Alice lives in Berlin.").await;
    assert!(redacted.contains("Alice"), "{redacted}");
}

/// One asserted language is authoritative: the detector is not called.
#[wasm_bindgen_test]
async fn one_asserted_language_skips_detection() {
    let never = "throw new Error('called');";
    let pipeline = ok(routed_to_german(never).with_languages(vec!["de".to_owned()]));
    let (redacted, _) = run_text(&pipeline, "Alice lives in Berlin.").await;
    assert!(!redacted.contains("Alice"), "{redacted}");
}

/// Several asserted languages are the detector's candidates.
#[wasm_bindgen_test]
async fn several_asserted_languages_are_the_candidates() {
    let detect = "if (candidates.join() !== 'fr,de') throw new Error(candidates.join()); \
                  return [{ language: 'de', confidence: 0.6 }];";
    let pipeline =
        ok(routed_to_german(detect).with_languages(vec!["fr".to_owned(), "de".to_owned()]));
    run_text(&pipeline, "Alice lives in Berlin.").await;
}

/// A detection with an invalid tag or span fails the run.
#[wasm_bindgen_test]
async fn an_invalid_detection_fails_the_run() {
    for detect in [
        "return [{ language: 'not a tag', confidence: 1 }];",
        "return [{ language: 'de', confidence: 1, start: 4, end: 999 }];",
        "return [{ language: 'de', confidence: 1, start: 4 }];",
    ] {
        let error = err(routed_to_german(detect)
            .redact(b"Alice wrote this.".to_vec(), Some("txt".to_owned()))
            .await);
        assert_eq!(error.kind(), ElideErrorKind::MalformedInput, "{detect}");
    }
}

/// A language-detection backend cannot be routed by language: a detection has
/// no language to route by.
#[wasm_bindgen_test]
fn a_language_routed_detector_is_refused() {
    let detector = Backend::callback(function("text, candidates", DETECTS));
    let routed = ok(detector
        .language_routed()
        .route("de".to_owned(), finds_nothing()));
    let error = err(Enricher::language(routed));
    assert_eq!(error.kind(), ElideErrorKind::Configuration);
}

/// The OCR callback gets the call's language with the image.
#[wasm_bindgen_test]
async fn the_ocr_callback_gets_the_language() {
    let ocr = function(
        "image, language",
        "if (language !== 'de') throw new Error('language ' + language); return [];",
    );
    let pipeline = ok(ok(Orchestrator::new().with(
        Analyzer::image()
            .enrich(Enricher::ocr(Backend::callback(ocr)))
            .recognize(common::patterns()),
        None,
    ))
    .with_languages(vec!["de".to_owned()]));
    common::run(&pipeline, PNG, Some("png")).await;
}

/// A rate of one call per interval holds the second call until the interval
/// has passed, on the browser's clock and timer.
#[wasm_bindgen_test]
async fn a_rate_limit_spaces_calls_out() {
    let backend =
        ok(finds_alice().rate_limited(config(r#"{ "count": 1, "intervalMs": 200, "burst": 1 }"#)));
    let pipeline = removing_found_by(Recognizer::ner(backend));
    let start = Date::now();
    run_text(&pipeline, "Alice wrote this.").await;
    run_text(&pipeline, "Alice wrote that.").await;
    let elapsed = Date::now() - start;
    assert!(elapsed >= 150.0, "two calls took {elapsed} ms");
}

/// A concurrency limit passes calls through.
#[wasm_bindgen_test]
async fn a_concurrency_limit_passes_calls_through() {
    let backend = ok(finds_alice().concurrency_limited(1));
    let (redacted, _) = run_text(
        &removing_found_by(Recognizer::ner(backend)),
        "Alice wrote this.",
    )
    .await;
    assert!(!redacted.contains("Alice"), "{redacted}");
}

/// Limits that would never admit a call, and invalid language tags, are
/// refused.
#[wasm_bindgen_test]
fn limits_that_admit_nothing_are_refused() {
    let zero_rate =
        err(finds_nothing().rate_limited(config(r#"{ "count": 0, "intervalMs": 1000 }"#)));
    assert_eq!(zero_rate.kind(), ElideErrorKind::Configuration);
    let zero_burst =
        err(finds_nothing()
            .rate_limited(config(r#"{ "count": 1, "intervalMs": 1000, "burst": 0 }"#)));
    assert_eq!(zero_burst.kind(), ElideErrorKind::Configuration);
    let zero_concurrency = err(finds_nothing().concurrency_limited(0));
    assert_eq!(zero_concurrency.kind(), ElideErrorKind::Configuration);
    let bad_tag = err(finds_nothing()
        .language_routed()
        .route("not a tag".to_owned(), finds_alice()));
    assert_eq!(bad_tag.kind(), ElideErrorKind::Configuration);
    let unrouted = err(finds_nothing().route("en".to_owned(), finds_alice()));
    assert_eq!(unrouted.kind(), ElideErrorKind::Configuration);
}
