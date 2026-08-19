//! Assertion helpers shared by the codec e2e tests: entity presence by
//! label/value, and redacted-output checks (originals gone, tokens in).
//!
//! These are `macro_rules!` macros rather than functions so a failure is
//! reported at the *test* call site (like `assert_eq!`). Their syntax leans on
//! the macro form: the value checks take **comma-separated needles** (no
//! `&[…]`) and borrow the haystack internally (no leading `&`), and the label
//! checks take **one or more labels** in a single call, replacing the
//! `for label in [
//! …] { … }` loops. They are re-exported (`pub(crate) use`) so a test still
//! brings them in with `use crate::support::asserts::assert_label_present;`.

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
            let found = entities.iter().any(|e| e.label == label);
            assert!(
                found,
                "expected an entity labeled {:?}; found {:?}",
                label,
                entities.iter().map(|e| e.label.clone()).collect::<Vec<_>>(),
            );
        )+
    }};
}

/// Assert that no detected entity carries any of the given labels — the
/// negative of [`assert_label_present!`], for precision cases where a weak or
/// context-free value must not be flagged at all. Accepts one or more labels.
#[macro_export]
macro_rules! assert_label_absent {
    ($entities:expr, $($label:expr),+ $(,)?) => {{
        let entities = &$entities;
        $(
            let label = $label;
            let found: Vec<_> = entities.iter().filter(|e| e.label == label).collect();
            assert!(
                found.is_empty(),
                "expected no entity labeled {:?}, but found {} ({:?})",
                label,
                found.len(),
                found.iter().map(|e| e.label.clone()).collect::<Vec<_>>(),
            );
        )+
    }};
}

/// Assert that none of the given originals survives in the redacted output.
///
/// ```ignore
/// assert_pii_removed!(out, "alice@example.com", "+1 (415) 555-0142");
/// ```
#[macro_export]
macro_rules! assert_pii_removed {
    ($redacted:expr, $($original:expr),+ $(,)?) => {{
        let redacted: &str = &$redacted;
        $(
            assert!(
                !redacted.contains($original),
                "redacted output still contains {:?}:\n{redacted}",
                $original,
            );
        )+
    }};
}

/// Assert that every given replacement token appears in the redacted output.
#[macro_export]
macro_rules! assert_tokens_present {
    ($redacted:expr, $($token:expr),+ $(,)?) => {{
        let redacted: &str = &$redacted;
        $(
            assert!(
                redacted.contains($token),
                "redacted output is missing token {:?}:\n{redacted}",
                $token,
            );
        )+
    }};
}

/// Assert that each given substring still appears verbatim (e.g. non-sensitive
/// structure that redaction must not touch).
#[macro_export]
macro_rules! assert_preserved {
    ($redacted:expr, $($keep:expr),+ $(,)?) => {{
        let redacted: &str = &$redacted;
        $(
            assert!(
                redacted.contains($keep),
                "redacted output lost expected content {:?}:\n{redacted}",
                $keep,
            );
        )+
    }};
}

// Re-export so `use crate::support::asserts::assert_label_present;` resolves
// the macro by its module path, keeping every existing import line working.
pub(crate) use assert_label_absent;
pub(crate) use assert_label_present;
pub(crate) use assert_pii_removed;
pub(crate) use assert_preserved;
pub(crate) use assert_tokens_present;
