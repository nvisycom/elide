//! End-to-end: shipped patterns + dictionaries against the
//! US-jurisdiction fixtures (`testdata/inputs/us/<domain>.txt`).
//!
//! Each test scans one US fixture through a recognizer wired
//! with every shipped pattern and dictionary, then asserts the
//! entities a real US document of that domain is expected to
//! surface (substring + label).

mod fixtures;

use elide_core::assert_label_present;
use elide_core::entity::builtins;
use elide_core::primitive::ConfidenceThreshold;
use fixtures::scan;

#[tokio::test]
async fn builtin_identity() {
    let (text, entities) = scan(include_str!("../testdata/inputs/us/identity.txt")).await;
    assert_match!(
        &text,
        &entities,
        builtins::GOVERNMENT_ID.to_ref(),
        "123-45-6789",
    );
    assert_match!(&text, &entities, builtins::TAX_ID.to_ref(), "912-71-1234");
    assert_match!(
        &text,
        &entities,
        builtins::DRIVERS_LICENSE.to_ref(),
        "D123-4567-8901",
    );
    assert_match!(
        &text,
        &entities,
        builtins::PASSPORT_NUMBER.to_ref(),
        "A12345678",
    );
    assert_match!(
        &text,
        &entities,
        builtins::POSTAL_CODE.to_ref(),
        "97477-1234",
    );
}

/// A bare nine-digit SSN is kept only beside a keyword: on its own it scores
/// below the default threshold, and "SSN:" lifts it over.
#[tokio::test]
async fn a_bare_ssn_needs_its_keyword() {
    let text = "SSN: 123456789.\n\nThe parcel left the warehouse on time as order 234567890.";
    let (text, entities) = scan(text).await;
    let confidence_of = |needle: &str| {
        entities
            .iter()
            .filter(|e| e.label() == &builtins::GOVERNMENT_ID.to_ref())
            .find(|e| {
                e.location()
                    .range()
                    .is_some_and(|r| text.get(r.start..r.end) == Some(needle))
            })
            .map(|e| e.confidence())
    };
    let threshold = ConfidenceThreshold::BASELINE;
    assert!(confidence_of("123456789").is_some_and(|c| threshold.passes(c)));
    assert!(confidence_of("234567890").is_none_or(|c| !threshold.passes(c)));
}

#[tokio::test]
async fn builtin_finance() {
    let (text, entities) = scan(include_str!("../testdata/inputs/us/finance.txt")).await;
    assert_match!(
        &text,
        &entities,
        builtins::BANK_ROUTING.to_ref(),
        "121000358",
    );
    // bank_account is `\b\d{8,17}\b` with score 0.05, it requires
    // a context-keyword boost (e.g. `account`) to clear the
    // confidence threshold. The fixture provides one.
    assert_label_present!(entities, builtins::BANK_ACCOUNT.to_ref());
}

#[tokio::test]
async fn builtin_health() {
    let (text, entities) = scan(include_str!("../testdata/inputs/us/health.txt")).await;
    assert_match!(
        &text,
        &entities,
        builtins::MEDICAL_ID.to_ref(),
        "1234567893",
    );
    assert_match!(
        &text,
        &entities,
        builtins::MEDICAL_ID.to_ref(),
        "1EG4-TE5-MK73",
    );
    assert_match!(&text, &entities, builtins::MEDICAL_ID.to_ref(), "BB0000000");
}
