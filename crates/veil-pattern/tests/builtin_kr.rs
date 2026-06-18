//! End-to-end: shipped patterns + dictionaries against the
//! KR-jurisdiction fixtures (`testdata/inputs/kr/<domain>.txt`).

mod fixtures;

use fixtures::{assert_label_present, assert_match, scan};
use veil_core::entity::builtins;

#[tokio::test]
async fn builtin_identity() {
    let (text, entities) = scan(include_str!("../testdata/inputs/kr/identity.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::GOVERNMENT_ID.to_ref(),
        "800101-1112343",
    );
    assert_match(
        &text,
        &entities,
        builtins::GOVERNMENT_ID.to_ref(),
        "900101-5112344",
    );
    assert_match(
        &text,
        &entities,
        builtins::PASSPORT_NUMBER.to_ref(),
        "M123N4567",
    );
    assert_match(
        &text,
        &entities,
        builtins::DRIVERS_LICENSE.to_ref(),
        "11-20-123456-78",
    );
}

#[tokio::test]
async fn builtin_finance() {
    let (text, entities) = scan(include_str!("../testdata/inputs/kr/finance.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::COMPANY_ID.to_ref(),
        "123-45-67891",
    );
    assert_label_present(&entities, builtins::COMPANY_ID.to_ref());
}
