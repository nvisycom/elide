//! Assertion helpers shared by the codec e2e tests: redacted-output checks
//! (originals gone, tokens in, structure preserved).
//!
//! These are `macro_rules!` macros rather than functions so a failure is
//! reported at the *test* call site (like `assert_eq!`). The value checks take
//! **comma-separated needles** (no `&[…]`) and borrow the haystack internally
//! (no leading `&`). They are re-exported (`pub(crate) use`) so a test still
//! brings them in with `use crate::support::asserts::assert_pii_removed;`.
//!
//! The entity-label assertions (`assert_label_present!` / `assert_label_absent!`)
//! live in `elide_core` (behind its `fixtures` feature); tests import those
//! directly with `use elide_core::assert_label_present;`.

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
macro_rules! assert_content_preserved {
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

// Re-export so `use crate::support::asserts::assert_pii_removed;` resolves the
// macro by its module path. The entity-label macros live in `elide_core`;
// tests import those directly from there.
pub(crate) use assert_content_preserved;
pub(crate) use assert_pii_removed;
pub(crate) use assert_tokens_present;
