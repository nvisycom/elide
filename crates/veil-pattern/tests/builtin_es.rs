//! End-to-end: shipped patterns + dictionaries against the
//! ES-jurisdiction fixtures (`testdata/inputs/es/<domain>.txt`).
//!
//! Each test scans one ES fixture through a recognizer wired
//! with every shipped pattern and dictionary, then asserts the
//! entities a real Spanish document of that domain is expected
//! to surface (substring + label).

mod fixtures;

use fixtures::{assert_label_present, assert_match, scan};
use veil_core::entity::builtins;

#[tokio::test]
async fn builtin_identity() {
    let (text, entities) = scan(include_str!("../testdata/inputs/es/identity.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::GOVERNMENT_ID.to_ref(),
        "12345678Z",
    );
    assert_match(
        &text,
        &entities,
        builtins::GOVERNMENT_ID.to_ref(),
        "X1234567L",
    );
    assert_match(
        &text,
        &entities,
        builtins::PASSPORT_NUMBER.to_ref(),
        "AAA123456",
    );
}

#[tokio::test]
async fn builtin_finance() {
    let (text, entities) = scan(include_str!("../testdata/inputs/es/finance.txt")).await;
    assert_match(&text, &entities, builtins::COMPANY_ID.to_ref(), "A12345674");
}

#[tokio::test]
async fn builtin_contact() {
    let (text, entities) = scan(include_str!("../testdata/inputs/es/contact.txt")).await;
    assert_match(&text, &entities, builtins::POSTAL_CODE.to_ref(), "28013");
    // English-language nationality dictionary stays silent on a
    // Spanish document — assert it didn't fire.
    assert!(
        !entities
            .iter()
            .any(|e| e.label == builtins::NATIONALITY.to_ref()),
        "english-language NATIONALITY dictionary should not match on an ES fixture",
    );
    assert_label_present(&entities, builtins::POSTAL_CODE.to_ref());
}
