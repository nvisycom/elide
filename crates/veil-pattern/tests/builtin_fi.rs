//! End-to-end: shipped patterns + dictionaries against the
//! FI-jurisdiction fixtures (`testdata/inputs/fi/<domain>.txt`).

mod fixtures;

use fixtures::{assert_label_present, assert_match, scan};
use nvisy_core::entity::builtins;

#[tokio::test]
async fn builtin_identity() {
    let (text, entities) = scan(include_str!("../testdata/inputs/fi/identity.txt")).await;
    assert_match(
        &text,
        &entities,
        builtins::GOVERNMENT_ID.label_ref(),
        "010170-123F",
    );
    assert_label_present(&entities, builtins::GOVERNMENT_ID.label_ref());
}
