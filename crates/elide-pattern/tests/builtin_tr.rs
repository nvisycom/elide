//! End-to-end: shipped patterns + dictionaries against the
//! TR-jurisdiction fixtures (`testdata/inputs/tr/<domain>.txt`).

mod fixtures;

use elide_core::entity::builtins;
use fixtures::{assert_label_present, assert_match, scan};

#[tokio::test]
async fn builtin_identity() {
    let (text, entities) = scan(include_str!("../testdata/inputs/tr/identity.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::GOVERNMENT_ID.to_ref(),
        "12345678950",
    );
    assert_match(
        &text,
        &entities,
        builtins::LICENSE_PLATE.to_ref(),
        "34-ABC-1234",
    );
}

#[tokio::test]
async fn builtin_contact() {
    let (text, entities) = scan(include_str!("../testdata/inputs/tr/contact.txt")).await;
    assert_match(&text, &entities, builtins::POSTAL_CODE.to_ref(), "34000");
    assert_label_present(&entities, builtins::POSTAL_CODE.to_ref());
}
