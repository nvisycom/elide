//! End-to-end: shipped patterns + dictionaries against the
//! IN-jurisdiction fixtures (`testdata/inputs/in/<domain>.txt`).
//!
//! Each test scans one IN fixture through a recognizer wired
//! with every shipped pattern and dictionary, then asserts the
//! entities a real Indian document of that domain is expected
//! to surface (substring + label).

mod fixtures;

use fixtures::{assert_label_present, assert_match, scan};
use veil_core::entity::builtins;

#[tokio::test]
async fn builtin_identity() {
    let (text, entities) = scan(include_str!("../testdata/inputs/in/identity.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::GOVERNMENT_ID.label_ref(),
        "2341 2341 2346",
    );
    assert_match(&text, &entities, builtins::TAX_ID.label_ref(), "ABCPK1234E");
    assert_match(
        &text,
        &entities,
        builtins::PASSPORT_NUMBER.label_ref(),
        "M1234567",
    );
    assert_match(
        &text,
        &entities,
        builtins::GOVERNMENT_ID.label_ref(),
        "ABC1234567",
    );
}

#[tokio::test]
async fn builtin_finance() {
    let (text, entities) = scan(include_str!("../testdata/inputs/in/finance.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::TAX_ID.label_ref(),
        "27AAAPL1234C1ZE",
    );
    assert_match(&text, &entities, builtins::TAX_ID.label_ref(), "AAAPL1234C");
}

#[tokio::test]
async fn builtin_vehicle() {
    let (text, entities) = scan(include_str!("../testdata/inputs/in/vehicle.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::LICENSE_PLATE.label_ref(),
        "MH12AB1234",
    );
    assert_label_present(&entities, builtins::LICENSE_PLATE.label_ref());
}
