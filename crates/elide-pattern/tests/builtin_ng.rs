//! End-to-end: shipped patterns + dictionaries against the
//! NG-jurisdiction fixtures (`testdata/inputs/ng/<domain>.txt`).

mod fixtures;

use fixtures::{assert_label_present, assert_match, scan};
use elide_core::entity::builtins;

#[tokio::test]
async fn builtin_identity() {
    let (text, entities) = scan(include_str!("../testdata/inputs/ng/identity.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::GOVERNMENT_ID.to_ref(),
        "12345678902",
    );
}

#[tokio::test]
async fn builtin_vehicle() {
    let (text, entities) = scan(include_str!("../testdata/inputs/ng/vehicle.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::LICENSE_PLATE.to_ref(),
        "ABC-123DE",
    );
    assert_label_present(&entities, builtins::LICENSE_PLATE.to_ref());
}
