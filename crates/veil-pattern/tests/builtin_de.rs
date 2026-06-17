//! End-to-end: shipped patterns + dictionaries against the
//! DE-jurisdiction fixtures (`testdata/inputs/de/<domain>.txt`).
//!
//! Each test scans one DE fixture through a recognizer wired
//! with every shipped pattern and dictionary, then asserts the
//! entities a real German document of that domain is expected to
//! surface (substring + label).

mod fixtures;

use fixtures::{assert_label_present, assert_match, scan};
use nvisy_core::entity::builtins;

#[tokio::test]
async fn builtin_identity() {
    let (text, entities) = scan(include_str!("../testdata/inputs/de/identity.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::GOVERNMENT_ID.label_ref(),
        "L01X00T44",
    );
    assert_match(
        &text,
        &entities,
        builtins::PASSPORT_NUMBER.label_ref(),
        "C0J9H58P3",
    );
    assert_match(
        &text,
        &entities,
        builtins::TAX_ID.label_ref(),
        "65929970489",
    );
    assert_match(
        &text,
        &entities,
        builtins::TAX_ID.label_ref(),
        "DE129273398",
    );
    assert_match(
        &text,
        &entities,
        builtins::DRIVERS_LICENSE.label_ref(),
        "B072RRE2I52",
    );
    assert_match(
        &text,
        &entities,
        builtins::NATIONAL_INSURANCE_NUMBER.label_ref(),
        "15010685M016",
    );
}

#[tokio::test]
async fn builtin_health() {
    let (text, entities) = scan(include_str!("../testdata/inputs/de/health.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::MEDICAL_ID.label_ref(),
        "381789045",
    );
    assert_match(
        &text,
        &entities,
        builtins::MEDICAL_ID.label_ref(),
        "123456601",
    );
    assert_match(
        &text,
        &entities,
        builtins::INSURANCE_ID.label_ref(),
        "A000500015",
    );
}

#[tokio::test]
async fn builtin_vehicle() {
    let (text, entities) = scan(include_str!("../testdata/inputs/de/vehicle.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::LICENSE_PLATE.label_ref(),
        "B-AB 1234",
    );
}

#[tokio::test]
async fn builtin_contact() {
    let (text, entities) = scan(include_str!("../testdata/inputs/de/contact.txt")).await;
    assert_match(&text, &entities, builtins::POSTAL_CODE.label_ref(), "10117");
    // English-language nationality dictionary stays silent on a
    // German document — assert it didn't fire.
    assert!(
        !entities
            .iter()
            .any(|e| e.label == builtins::NATIONALITY.label_ref()),
        "english-language NATIONALITY dictionary should not match on a DE fixture",
    );
    // Sanity: at least one PLZ entity surfaced.
    assert_label_present(&entities, builtins::POSTAL_CODE.label_ref());
}

#[tokio::test]
async fn builtin_finance() {
    let (text, entities) = scan(include_str!("../testdata/inputs/de/finance.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::COMPANY_ID.label_ref(),
        "HRB 123456",
    );
}
