//! End-to-end: shipped patterns + dictionaries against the
//! AU-jurisdiction fixtures (`testdata/inputs/au/<domain>.txt`).
//!
//! Each test scans one AU fixture through a recognizer wired
//! with every shipped pattern and dictionary, then asserts the
//! entities a real Australian document of that domain is
//! expected to surface (substring + label).

mod fixtures;

use fixtures::{assert_label_present, assert_match, scan};
use nvisy_core::entity::builtins;

#[tokio::test]
async fn builtin_identity() {
    let (text, entities) = scan(include_str!("../testdata/inputs/au/identity.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::TAX_ID.label_ref(),
        "123 456 782",
    );
}

#[tokio::test]
async fn builtin_finance() {
    let (text, entities) = scan(include_str!("../testdata/inputs/au/finance.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::COMPANY_ID.label_ref(),
        "51 824 753 556",
    );
    assert_match(
        &text,
        &entities,
        builtins::COMPANY_ID.label_ref(),
        "123 456 780",
    );
    assert_label_present(&entities, builtins::COMPANY_ID.label_ref());
}

#[tokio::test]
async fn builtin_health() {
    let (text, entities) = scan(include_str!("../testdata/inputs/au/health.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::INSURANCE_ID.label_ref(),
        "2228 12366 1",
    );
}
