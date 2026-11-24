//! The redaction operators beyond replace, mask, remove, cover and keep, run
//! end to end through the bindings, and the stages that refuse them.

#![cfg(target_arch = "wasm32")]

mod common;

use elide_wasm::analyzer::Analyzer;
use elide_wasm::anonymizer::Anonymizer;
use elide_wasm::error::ElideErrorKind;
use elide_wasm::operator::Operator;
use elide_wasm::orchestrator::Orchestrator;
use elide_wasm::recognizer::Recognizer;
use elide_wasm::rule::Rule;
use wasm_bindgen_test::wasm_bindgen_test;

use self::common::{config, err, finds, ok, patterns, run, run_text};

/// The email the text tests redact.
const EMAIL: &str = "jane.doe@example.com";

/// A 32-byte key.
const KEY: [u8; 32] = [7; 32];

/// What `operator` makes of `value`, found as a `label` in a sentence.
async fn applied(operator: Operator, value: &str, label: &str) -> String {
    let pipeline = ok(Orchestrator::new().with(
        Analyzer::text().recognize(Recognizer::ner(finds(value, label))),
        Some(Anonymizer::text().rule(Rule::fallback(operator))),
    ));
    let (redacted, _) = run_text(&pipeline, &format!("[{value}]")).await;
    redacted
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or_else(|| panic!("the brackets were redacted: {redacted}"))
        .to_owned()
}

/// Whether `value` is `len` lowercase hex digits.
fn is_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[wasm_bindgen_test]
async fn hmac_replaces_a_value_by_its_keyed_hash() {
    let sha256 = applied(
        ok(Operator::hmac(KEY.to_vec(), None)),
        EMAIL,
        "email_address",
    )
    .await;
    assert!(is_hex(&sha256, 64), "{sha256}");
    let again = applied(
        ok(Operator::hmac(KEY.to_vec(), None)),
        EMAIL,
        "email_address",
    )
    .await;
    assert_eq!(
        sha256, again,
        "the same key hashes the same value the same way"
    );
    let sha512 = applied(
        ok(Operator::hmac(
            KEY.to_vec(),
            Some(config(r#"{ "algorithm": "sha512" }"#)),
        )),
        EMAIL,
        "email_address",
    )
    .await;
    assert!(is_hex(&sha512, 128), "{sha512}");
}

#[wasm_bindgen_test]
async fn hash_replaces_a_value_by_its_salted_hash() {
    let plain = applied(ok(Operator::hash(None)), EMAIL, "email_address").await;
    assert!(is_hex(&plain, 64), "{plain}");
    let salted = applied(
        ok(Operator::hash(Some(config(r#"{ "salt": [1, 2, 3] }"#)))),
        EMAIL,
        "email_address",
    )
    .await;
    assert!(is_hex(&salted, 64), "{salted}");
    assert_ne!(plain, salted, "the salt changes the hash");
}

#[wasm_bindgen_test]
async fn encrypt_replaces_a_value_by_its_encryption() {
    let first = applied(ok(Operator::encrypt(KEY.to_vec())), EMAIL, "email_address").await;
    let second = applied(ok(Operator::encrypt(KEY.to_vec())), EMAIL, "email_address").await;
    assert!(!first.contains(EMAIL) && !first.is_empty(), "{first}");
    assert_ne!(first, second, "each encryption draws a fresh nonce");
}

#[wasm_bindgen_test]
fn encrypt_refuses_a_key_that_is_not_32_bytes() {
    let error = err(Operator::encrypt(vec![7; 31]));
    assert_eq!(error.kind(), ElideErrorKind::Configuration);
    let error = err(Operator::hmac(Vec::new(), None));
    assert_eq!(error.kind(), ElideErrorKind::Configuration);
}

#[wasm_bindgen_test]
async fn fake_replaces_a_value_by_a_fake_of_its_label() {
    let fake = || {
        ok(Operator::fake(
            Operator::remove(),
            Some(config(r#"{ "seed": 7 }"#)),
        ))
    };
    let faked = applied(fake(), EMAIL, "email_address").await;
    assert!(faked != EMAIL && faked.contains('@'), "{faked}");
    let unfakeable = applied(fake(), "felony theft", "criminal_charge").await;
    assert_eq!(unfakeable, "", "a label it cannot fake gets the fallback");
}

#[wasm_bindgen_test]
async fn generalize_date_keeps_the_unit_asked_for() {
    let year = applied(
        ok(Operator::generalize_date(None)),
        "1987-03-14",
        "date_of_birth",
    )
    .await;
    assert_eq!(year, "1987");
    let month = applied(
        ok(Operator::generalize_date(Some(config(
            r#"{ "granularity": "yearMonth", "style": "us" }"#,
        )))),
        "03/14/1987",
        "date_of_birth",
    )
    .await;
    assert_eq!(month, "03/1987");
}

#[wasm_bindgen_test]
async fn clamp_replaces_a_number_past_a_bound() {
    let clamp = || {
        ok(Operator::clamp(config(
            r#"{ "ceiling": { "value": 90 }, "floor": { "value": 18, "format": "under {n}" } }"#,
        )))
    };
    assert_eq!(applied(clamp(), "94", "age").await, "90+");
    assert_eq!(applied(clamp(), "12", "age").await, "under 18");
    assert_eq!(applied(clamp(), "40", "age").await, "40");
    assert_eq!(
        err(Operator::clamp(config("{}"))).kind(),
        ElideErrorKind::Configuration
    );
}

#[wasm_bindgen_test]
async fn truncate_keeps_the_head_and_tail() {
    let truncated = applied(
        ok(Operator::truncate(config(
            r#"{ "keepPrefix": 4, "keepSuffix": 4 }"#,
        ))),
        "4111111111111234",
        "payment_card",
    )
    .await;
    assert!(
        truncated.starts_with("4111") && truncated.ends_with("1234") && truncated.len() < 16,
        "{truncated}"
    );
}

/// `dropRow` removes the whole row of a matched cell.
#[wasm_bindgen_test]
async fn drop_row_removes_the_row_of_a_match() {
    let pipeline = ok(Orchestrator::new().with(
        Analyzer::tabular().recognize(patterns()),
        Some(Anonymizer::tabular().rule(Rule::fallback(Operator::drop_row()))),
    ));
    let csv = format!("name,contact\nJane,{EMAIL}\nBob,none\n");
    let (redacted, _) = run(&pipeline, csv.as_bytes(), Some("csv")).await;
    let redacted = String::from_utf8(redacted).unwrap();
    assert!(
        !redacted.contains("Jane") && redacted.contains("Bob"),
        "{redacted}"
    );
}

/// A clamp whose bounds are not finite, or whose floor is not below its
/// ceiling, is refused; `1e999` parses to infinity.
#[wasm_bindgen_test]
fn a_clamp_with_unusable_bounds_is_refused() {
    for bounds in [
        r#"{ "ceiling": { "value": 1e999 } }"#,
        r#"{ "floor": { "value": 100 }, "ceiling": { "value": 10 } }"#,
        r#"{ "floor": { "value": 10 }, "ceiling": { "value": 10 } }"#,
    ] {
        let error = err(Operator::clamp(config(bounds)));
        assert_eq!(error.kind(), ElideErrorKind::Configuration, "{bounds}");
    }
}

/// A fake whose fallback is not a text operator says so, even for an operator
/// that applies to the stage on its own.
#[wasm_bindgen_test]
fn a_fake_names_a_fallback_that_is_not_a_text_operator() {
    let error = err(Orchestrator::new().with(
        Analyzer::tabular(),
        Some(Anonymizer::tabular().rule(Rule::fallback(ok(Operator::fake(
            Operator::drop_row(),
            None,
        ))))),
    ));
    assert!(
        error.message().contains("fallback must be a text operator"),
        "{}",
        error.message()
    );
}

/// An operator offered to a stage it does not apply to is refused when the
/// stage is built, a fake's fallback included.
#[wasm_bindgen_test]
fn an_operator_is_refused_by_a_stage_it_does_not_apply_to() {
    let refused = |operator: Operator| {
        err(Orchestrator::new().with(
            Analyzer::text(),
            Some(Anonymizer::text().rule(Rule::fallback(operator))),
        ))
        .kind()
    };
    assert_eq!(
        refused(ok(Operator::blur(4.0))),
        ElideErrorKind::Configuration
    );
    assert_eq!(
        refused(ok(Operator::pixelate(8))),
        ElideErrorKind::Configuration
    );
    assert_eq!(
        refused(ok(Operator::beep(None))),
        ElideErrorKind::Configuration
    );
    assert_eq!(refused(Operator::drop_row()), ElideErrorKind::Configuration);
    let fake_of_blur = ok(Operator::fake(ok(Operator::blur(4.0)), None));
    assert_eq!(refused(fake_of_blur), ElideErrorKind::Configuration);
}
