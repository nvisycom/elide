//! End-to-end: shipped patterns + dictionaries against the
//! IT-jurisdiction fixtures (`testdata/inputs/it/<domain>.txt`).
//!
//! Each test scans one IT fixture through a recognizer wired
//! with every shipped pattern and dictionary, then asserts the
//! entities a real Italian document of that domain is expected
//! to surface (substring + label).

mod fixtures;

use fixtures::{assert_label_present, assert_match, scan};
use veil_core::entity::builtins;

#[tokio::test]
async fn builtin_identity() {
    let (text, entities) = scan(include_str!("../testdata/inputs/it/identity.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::TAX_ID.label_ref(),
        "RSSMRA70A01H501S",
    );
    assert_match(
        &text,
        &entities,
        builtins::GOVERNMENT_ID.label_ref(),
        "CA12345AB",
    );
    assert_match(
        &text,
        &entities,
        builtins::PASSPORT_NUMBER.label_ref(),
        "YA1234567",
    );
    assert_match(
        &text,
        &entities,
        builtins::DRIVERS_LICENSE.label_ref(),
        "MI1234567A",
    );
}

#[tokio::test]
async fn builtin_finance() {
    let (text, entities) = scan(include_str!("../testdata/inputs/it/finance.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::TAX_ID.label_ref(),
        "00154980569",
    );
    // English-language nationality dictionary stays silent on an
    // Italian document — assert it didn't fire.
    assert!(
        !entities
            .iter()
            .any(|e| e.label == builtins::NATIONALITY.label_ref()),
        "english-language NATIONALITY dictionary should not match on an IT fixture",
    );
    assert_label_present(&entities, builtins::TAX_ID.label_ref());
}
