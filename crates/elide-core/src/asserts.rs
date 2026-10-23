//! Entity-assertion macros shared by the test suites of this crate and the
//! downstream crates that detect entities.
//!
//! They are `macro_rules!` rather than functions so a failure is reported at
//! the *test* call site (like `assert_eq!`), and each takes one or more labels
//! in a single call. They operate on any slice of
//! [`Entity<M>`](crate::entity::Entity), so a text, tabular, or image suite
//! uses the same assertions.
//!
//! Gated behind the `fixtures` feature and intended for `[dev-dependencies]`:
//! `elide-core = { ..., features = ["fixtures"] }`, then
//! `use elide_core::{assert_label_present, assert_label_absent};`.

/// Assert that some detected entity carries each given label. Accepts one or
/// more labels; fails with the full label list on the first that is missing.
///
/// ```ignore
/// assert_label_present!(entities, EMAIL);
/// assert_label_present!(entities, EMAIL, PHONE, IBAN);
/// ```
#[macro_export]
macro_rules! assert_label_present {
    ($entities:expr, $($label:expr),+ $(,)?) => {{
        let entities = &$entities;
        $(
            let label = $label;
            assert!(
                entities.iter().any(|e| e.label == label),
                "expected an entity labeled {:?}; found {:?}",
                label,
                entities.iter().map(|e| e.label.clone()).collect::<Vec<_>>(),
            );
        )+
    }};
}

/// Assert that no detected entity carries any of the given labels — the
/// negative of [`assert_label_present!`], for precision cases where a weak or
/// shaped-but-not-real value must not be flagged at all. Accepts one or more
/// labels.
///
/// ```ignore
/// assert_label_absent!(entities, PHONE_NUMBER);
/// ```
#[macro_export]
macro_rules! assert_label_absent {
    ($entities:expr, $($label:expr),+ $(,)?) => {{
        let entities = &$entities;
        $(
            let label = $label;
            let found = entities.iter().filter(|e| e.label == label).count();
            assert!(
                found == 0,
                "expected no entity labeled {:?}, but found {found}",
                label,
            );
        )+
    }};
}
