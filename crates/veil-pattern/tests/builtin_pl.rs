//! End-to-end: shipped patterns + dictionaries against the
//! PL-jurisdiction fixtures (`testdata/inputs/pl/<domain>.txt`).
//!
//! Each test scans one PL fixture through a recognizer wired
//! with every shipped pattern and dictionary, then asserts the
//! entities a real Polish document of that domain is expected
//! to surface (substring + label).

mod fixtures;

use fixtures::{assert_label_present, assert_match, scan};
use nvisy_core::entity::builtins;

#[tokio::test]
async fn builtin_identity() {
    let (text, entities) = scan(include_str!("../testdata/inputs/pl/identity.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::GOVERNMENT_ID.label_ref(),
        "44051401359",
    );
}

#[tokio::test]
async fn builtin_finance() {
    let (text, entities) = scan(include_str!("../testdata/inputs/pl/finance.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::TAX_ID.label_ref(),
        "106-000-00-62",
    );
    assert_match(
        &text,
        &entities,
        builtins::COMPANY_ID.label_ref(),
        "123456785",
    );
}

#[tokio::test]
async fn builtin_contact() {
    let (text, entities) = scan(include_str!("../testdata/inputs/pl/contact.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::POSTAL_CODE.label_ref(),
        "00-001",
    );
    // English-language nationality dictionary stays silent on a
    // Polish document — assert it didn't fire.
    assert!(
        !entities
            .iter()
            .any(|e| e.label == builtins::NATIONALITY.label_ref()),
        "english-language NATIONALITY dictionary should not match on a PL fixture",
    );
    assert_label_present(&entities, builtins::POSTAL_CODE.label_ref());
}
