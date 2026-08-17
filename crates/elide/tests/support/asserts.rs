//! Assertion helpers shared by the codec e2e tests: entity presence by
//! label/value, and redacted-output checks (originals gone, tokens in).

use elide::entity::{Entity, LabelRef};
use elide::modality::Modality;

/// Assert that some detected entity carries `label`. Fails with the full
/// label list when missing.
pub fn assert_label_present<M: Modality>(entities: &[Entity<M>], label: &LabelRef) {
    let found = entities.iter().any(|e| &e.label == label);
    assert!(
        found,
        "expected an entity labeled {label:?}; found {:?}",
        entities.iter().map(|e| e.label.clone()).collect::<Vec<_>>(),
    );
}

/// Assert that none of `originals` survives in the redacted output.
pub fn assert_pii_removed(redacted: &str, originals: &[&str]) {
    for original in originals {
        assert!(
            !redacted.contains(original),
            "redacted output still contains {original:?}:\n{redacted}",
        );
    }
}

/// Assert that every replacement `token` appears in the redacted output.
pub fn assert_tokens_present(redacted: &str, tokens: &[&str]) {
    for token in tokens {
        assert!(
            redacted.contains(token),
            "redacted output is missing token {token:?}:\n{redacted}",
        );
    }
}

/// Assert that each `preserved` substring still appears verbatim (e.g.
/// non-sensitive structure that redaction must not touch).
pub fn assert_preserved(redacted: &str, preserved: &[&str]) {
    for keep in preserved {
        assert!(
            redacted.contains(keep),
            "redacted output lost expected content {keep:?}:\n{redacted}",
        );
    }
}
