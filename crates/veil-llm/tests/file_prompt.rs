//! End-to-end: load [`FilePrompt`] from a TOML fixture (Jinja2
//! template + label map + ignore list), then exercise both halves of
//! the [`Prompt`] trait: render against a populated
//! [`RecognizerInput`] and lift a stub [`LlmResponse`] back into
//! entities.
//!
//! Covers the minijinja code path end-to-end (variable interpolation,
//! `{% for %}`, `{% if %}`, the `| join` filter, hint snippet
//! rendering for text, bbox access for image), plus the
//! `label_map` / `labels_to_ignore` policy on the lift side.

use veil_core::entity::{LabelRef, builtins};
use veil_core::modality::image::{Image, ImageData, ImageLocation};
use veil_core::modality::text::{Text, TextData, TextLocation};
use veil_core::primitive::{BoundingBox, Dimensions, Point};
use veil_core::recognition::{Hint, RecognizerInput};
use veil_llm::backend::LlmResponse;
use veil_llm::{FilePrompt, Prompt};
const NER_TOML: &str = include_str!("../testdata/prompts/ner.toml");
const VLM_TOML: &str = include_str!("../testdata/prompts/vlm.toml");

#[test]
fn text_prompt_renders_template_and_lifts_entities() {
    let prompt = FilePrompt::<Text>::from_toml(NER_TOML).expect("ner.toml parses");

    // Realistic doc text. Hint coordinates pick out "Alice Carter".
    let body = "From: Alice Carter <alice.carter@acme.test>\nSubject: hello";
    let alice_start = body.find("Alice Carter").expect("alice substring");
    let alice_end = alice_start + "Alice Carter".len();

    let hint = Hint::<Text>::new(TextLocation::new(alice_start, alice_end))
        .with_name("uploader-alice")
        .with_label(builtins::PERSON_NAME.to_ref());

    let input = RecognizerInput::new(TextData::new(body))
        .with_hints(vec![hint])
        .with_labels(vec!["medical".to_owned(), "gdpr-request".to_owned()]);

    // -- build()
    let rendered = prompt.build(&input);
    assert!(rendered.contains(body), "source text missing: {rendered}");
    assert!(
        rendered.contains("Document labels: medical, gdpr-request"),
        "labels `| join` filter not applied: {rendered}",
    );
    assert!(
        rendered.contains("Uploader hints:"),
        "{{% if hints %}} branch not taken: {rendered}",
    );
    assert!(
        rendered.contains("uploader-alice (person_name): value=Alice Carter"),
        "{{% for %}} over hints didn't render name/kind/value: {rendered}",
    );
    assert!(
        rendered
            .contains("snippet=\"From: Alice Carter <alice.carter@acme.test>\nSubject: hello\""),
        "hint snippet not rendered (full body is within ±80 chars): {rendered}",
    );

    // -- lift(): the TOML maps `person_name → date_of_birth` and
    // ignores `diagnosis`. The model emits snake_case label names
    // (TextCandidate.entity_type is `Option<String>`); we expect
    // person_name → date_of_birth via the map, email_address
    // untouched, and diagnosis dropped by the ignore list.
    let response = LlmResponse::new(
        r#"{"entities":[
            {"entity_type":"person_name","value":"Alice Carter","context":"From: Alice Carter <","confidence":0.9},
            {"entity_type":"email_address","value":"alice.carter@acme.test","context":"<alice.carter@acme.test>","confidence":0.95},
            {"entity_type":"diagnosis","value":"hello","context":"Subject: hello","confidence":0.1}
        ]}"#,
    );
    let entities = prompt.lift(&response, &input);

    let kinds: Vec<LabelRef> = entities.iter().map(|e| e.label.clone()).collect();
    assert!(
        kinds.contains(&builtins::DATE_OF_BIRTH.to_ref()),
        "person_name should have been remapped to DateOfBirth via label_map: {kinds:?}",
    );
    assert!(
        kinds.contains(&builtins::EMAIL_ADDRESS.to_ref()),
        "email_address (no map entry) should pass through: {kinds:?}",
    );
    assert!(
        !kinds.contains(&builtins::DIAGNOSIS.to_ref()),
        "diagnosis was in labels_to_ignore but appeared: {kinds:?}",
    );
    assert_eq!(
        entities.len(),
        2,
        "expected 2 entities (diagnosis dropped), got {}: {kinds:?}",
        entities.len(),
    );
}

#[test]
fn image_prompt_renders_template_and_lifts_entities() {
    let prompt = FilePrompt::<Image>::from_toml(VLM_TOML).expect("vlm.toml parses");

    // Tiny PNG-shaped payload; the prompt only base64-encodes it.
    let bytes = b"\x89PNG\r\n\x1a\nfake-image-bytes".to_vec();
    let dims = Dimensions::new(640, 480);

    let hint = Hint::<Image>::new(ImageLocation::new(BoundingBox::from_origin_size(
        Point::new(10.0, 20.0),
        100.0,
        50.0,
    )))
    .with_name("uploader-face")
    .with_label(builtins::PERSON_NAME.to_ref());

    let input = RecognizerInput::new(ImageData::new(bytes.clone(), dims))
        .with_hints(vec![hint])
        .with_labels(vec!["badge".to_owned()]);

    // -- build()
    let rendered = prompt.build(&input);
    let expected_b64 = base64_encode(&bytes);
    assert!(
        rendered.contains(&expected_b64),
        "image_b64 missing from render: {rendered}",
    );
    assert!(
        rendered.contains("Tags: badge"),
        "labels `| join` not rendered: {rendered}",
    );
    assert!(
        rendered.contains("uploader-face (person_name): bbox=[10.0,20.0,100.0x50.0]"),
        "{{% for %}} over image hints with nested bbox access broken: {rendered}",
    );

    // -- lift(): label_map maps `person_name → date_of_birth`, and
    // labels_to_ignore drops `url`. The VLM emits typed snake_case
    // kinds; assert remap + ignore both fire.
    let response = LlmResponse::new(
        r#"{"entities":[
            {"label":"person_name","x":0.1,"y":0.1,"width":0.2,"height":0.2,"confidence":0.85},
            {"label":"license_plate","x":0.5,"y":0.5,"width":0.1,"height":0.05,"confidence":0.7},
            {"label":"url","x":0.0,"y":0.0,"width":0.05,"height":0.05,"confidence":0.9}
        ]}"#,
    );
    let entities = prompt.lift(&response, &input);

    let kinds: Vec<LabelRef> = entities.iter().map(|e| e.label.clone()).collect();
    assert!(
        kinds.contains(&builtins::DATE_OF_BIRTH.to_ref()),
        "person_name should have been remapped to DateOfBirth via label_map: {kinds:?}",
    );
    assert!(
        kinds.contains(&builtins::LICENSE_PLATE.to_ref()),
        "license_plate (no map entry) should pass through: {kinds:?}",
    );
    assert!(
        !kinds.contains(&builtins::URL.to_ref()),
        "url was in labels_to_ignore but appeared: {kinds:?}",
    );
    assert_eq!(
        entities.len(),
        2,
        "expected 2 entities (url dropped), got {}: {kinds:?}",
        entities.len(),
    );
}

/// Local base64 encoder so the test asserts on the exact string the
/// prompt rendered, without pulling in `base64` as a dev-dep.
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        if chunk.len() >= 2 {
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() == 3 {
            out.push(TABLE[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}
