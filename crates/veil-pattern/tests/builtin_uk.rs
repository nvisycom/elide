//! End-to-end: shipped patterns + dictionaries against the
//! UK-jurisdiction fixtures (`testdata/inputs/uk/<domain>.txt`).
//!
//! Each test scans one UK fixture through a recognizer wired
//! with every shipped pattern and dictionary, then asserts the
//! entities a real UK document of that domain is expected to
//! surface (substring + label).

mod fixtures;

use fixtures::{assert_label_present, assert_match, scan};
use nvisy_core::entity::builtins;

#[tokio::test]
async fn builtin_identity() {
    let (text, entities) = scan(include_str!("../testdata/inputs/uk/identity.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::MEDICAL_ID.label_ref(),
        "943 476 5919",
    );
    assert_match(
        &text,
        &entities,
        builtins::NATIONAL_INSURANCE_NUMBER.label_ref(),
        "AB123456C",
    );
    assert_match(
        &text,
        &entities,
        builtins::DRIVERS_LICENSE.label_ref(),
        "MORGA753116SM9IJ",
    );
    assert_match(
        &text,
        &entities,
        builtins::PASSPORT_NUMBER.label_ref(),
        "AB1234567",
    );
    // World nationality dictionary activates on UK text ("British").
    assert_label_present(&entities, builtins::NATIONALITY.label_ref());
}

#[tokio::test]
async fn builtin_contact() {
    let (text, entities) = scan(include_str!("../testdata/inputs/uk/contact.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::POSTAL_CODE.label_ref(),
        "SW1A 2AA",
    );
}

#[tokio::test]
async fn builtin_vehicle() {
    let (text, entities) = scan(include_str!("../testdata/inputs/uk/vehicle.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::LICENSE_PLATE.label_ref(),
        "AB51 ABC",
    );
}
