//! End-to-end: shipped patterns + dictionaries against the
//! SG-jurisdiction fixtures (`testdata/inputs/sg/<domain>.txt`).

mod fixtures;

use fixtures::{assert_label_present, assert_match, scan};
use veil_core::entity::builtins;

#[tokio::test]
async fn builtin_identity() {
    let (text, entities) = scan(include_str!("../testdata/inputs/sg/identity.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::GOVERNMENT_ID.to_ref(),
        "S2740116C",
    );
    assert_match(
        &text,
        &entities,
        builtins::GOVERNMENT_ID.to_ref(),
        "F1234567N",
    );
}

#[tokio::test]
async fn builtin_finance() {
    let (text, entities) = scan(include_str!("../testdata/inputs/sg/finance.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::COMPANY_ID.to_ref(),
        "200512345R",
    );
}

#[tokio::test]
async fn builtin_contact() {
    let (text, entities) = scan(include_str!("../testdata/inputs/sg/contact.txt")).await;
    assert_match(&text, &entities, builtins::POSTAL_CODE.to_ref(), "018989");
    assert_label_present(&entities, builtins::POSTAL_CODE.to_ref());
}
