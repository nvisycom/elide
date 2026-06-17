//! End-to-end: shipped patterns + dictionaries against the
//! SE-jurisdiction fixtures (`testdata/inputs/se/<domain>.txt`).

mod fixtures;

use fixtures::{assert_label_present, assert_match, scan};
use nvisy_core::entity::builtins;

#[tokio::test]
async fn builtin_identity() {
    let (text, entities) = scan(include_str!("../testdata/inputs/se/identity.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::GOVERNMENT_ID.label_ref(),
        "900101-1239",
    );
}

#[tokio::test]
async fn builtin_finance() {
    let (text, entities) = scan(include_str!("../testdata/inputs/se/finance.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::COMPANY_ID.label_ref(),
        "556677-1233",
    );
}

#[tokio::test]
async fn builtin_contact() {
    let (text, entities) = scan(include_str!("../testdata/inputs/se/contact.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::POSTAL_CODE.label_ref(),
        "111 60",
    );
    assert_label_present(&entities, builtins::POSTAL_CODE.label_ref());
}
