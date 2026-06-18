//! End-to-end: shipped patterns + dictionaries against the
//! TH-jurisdiction fixtures (`testdata/inputs/th/<domain>.txt`).

mod fixtures;

use elide_core::entity::builtins;
use fixtures::{assert_label_present, assert_match, scan};

#[tokio::test]
async fn builtin_identity() {
    let (text, entities) = scan(include_str!("../testdata/inputs/th/identity.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::GOVERNMENT_ID.to_ref(),
        "1-2345-67890-12-1",
    );
}

#[tokio::test]
async fn builtin_contact() {
    let (text, entities) = scan(include_str!("../testdata/inputs/th/contact.txt")).await;
    assert_match(&text, &entities, builtins::POSTAL_CODE.to_ref(), "10500");
    assert_label_present(&entities, builtins::POSTAL_CODE.to_ref());
}
