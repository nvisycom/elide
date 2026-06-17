//! End-to-end: shipped patterns + dictionaries against the
//! TH-jurisdiction fixtures (`testdata/inputs/th/<domain>.txt`).

mod fixtures;

use fixtures::{assert_label_present, assert_match, scan};
use veil_core::entity::builtins;

#[tokio::test]
async fn builtin_identity() {
    let (text, entities) = scan(include_str!("../testdata/inputs/th/identity.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::GOVERNMENT_ID.label_ref(),
        "1-2345-67890-12-1",
    );
}

#[tokio::test]
async fn builtin_contact() {
    let (text, entities) = scan(include_str!("../testdata/inputs/th/contact.txt")).await;
    assert_match(&text, &entities, builtins::POSTAL_CODE.label_ref(), "10500");
    assert_label_present(&entities, builtins::POSTAL_CODE.label_ref());
}
