//! End-to-end: shipped patterns + dictionaries against the
//! US-jurisdiction fixtures (`testdata/inputs/us/<domain>.txt`).
//!
//! Each test scans one US fixture through a recognizer wired
//! with every shipped pattern and dictionary, then asserts the
//! entities a real US document of that domain is expected to
//! surface (substring + label).

mod fixtures;

use elide_core::entity::builtins;
use fixtures::{assert_label_present, assert_match, scan};

#[tokio::test]
async fn builtin_identity() {
    let (text, entities) = scan(include_str!("../testdata/inputs/us/identity.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::GOVERNMENT_ID.to_ref(),
        "123-45-6789",
    );
    assert_match(&text, &entities, builtins::TAX_ID.to_ref(), "912-71-1234");
    assert_match(
        &text,
        &entities,
        builtins::DRIVERS_LICENSE.to_ref(),
        "D123-4567-8901",
    );
    assert_match(
        &text,
        &entities,
        builtins::PASSPORT_NUMBER.to_ref(),
        "A12345678",
    );
    assert_match(
        &text,
        &entities,
        builtins::POSTAL_CODE.to_ref(),
        "97477-1234",
    );
}

#[tokio::test]
async fn builtin_finance() {
    let (text, entities) = scan(include_str!("../testdata/inputs/us/finance.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::BANK_ROUTING.to_ref(),
        "121000358",
    );
    // bank_account is `\b\d{8,17}\b` with score 0.05 — it requires
    // a context-keyword boost (e.g. `account`) to clear the
    // confidence threshold. The fixture provides one.
    assert_label_present(&entities, builtins::BANK_ACCOUNT.to_ref());
}

#[tokio::test]
async fn builtin_health() {
    let (text, entities) = scan(include_str!("../testdata/inputs/us/health.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::MEDICAL_ID.to_ref(),
        "1234567893",
    );
    assert_match(
        &text,
        &entities,
        builtins::MEDICAL_ID.to_ref(),
        "1EG4-TE5-MK73",
    );
    assert_match(&text, &entities, builtins::MEDICAL_ID.to_ref(), "BB0000000");
}
