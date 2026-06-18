//! End-to-end: shipped patterns + dictionaries against the
//! CA-jurisdiction fixtures (`testdata/inputs/ca/<domain>.txt`).
//!
//! Each test scans one CA fixture through a recognizer wired
//! with every shipped pattern and dictionary, then asserts the
//! entities a real Canadian document of that domain is expected
//! to surface (substring + label).

mod fixtures;

use elide_core::entity::builtins;
use fixtures::{assert_label_present, assert_match, scan};

#[tokio::test]
async fn builtin_identity() {
    let (text, entities) = scan(include_str!("../testdata/inputs/ca/identity.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::GOVERNMENT_ID.to_ref(),
        "123 456 782",
    );
}

#[tokio::test]
async fn builtin_contact() {
    let (text, entities) = scan(include_str!("../testdata/inputs/ca/contact.txt")).await;
    assert_match(&text, &entities, builtins::POSTAL_CODE.to_ref(), "K1P 1A1");
    assert_label_present(&entities, builtins::POSTAL_CODE.to_ref());
}
