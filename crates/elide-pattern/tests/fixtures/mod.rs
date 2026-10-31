//! Shared helpers for the `builtin_*` end-to-end test suites.
//!
//! Each per-region test file (`tests/builtin.rs`,
//! `tests/builtin_us.rs`, `tests/builtin_uk.rs`, …) declares this
//! module via `mod fixtures;` and calls [`scan`] + the `assert_*`
//! helpers to express expectations against a single shared
//! [`PatternRecognizer`] built from every shipped pattern and
//! dictionary.
//!
//! The label assertions (`assert_label_present!` / `assert_label_absent!`)
//! come from `elide_core` (behind its `fixtures` feature); only the
//! value-aware [`assert_match`] and the scan drivers are local here.
//!
//! The module is `include`d by each `builtin_*` binary, so a helper used by
//! only some of them (e.g. [`scan_in_country`]) is dead code in the rest; the
//! crate-wide allow keeps those binaries warning-free.

#![allow(dead_code)]

use elide_core::entity::Entity;
use elide_core::modality::text::{Text, TextData};
use elide_core::primitive::CountryCode;
use elide_core::recognition::{Context, Recognizer, Scope, Subject};
use elide_pattern::PatternRecognizer;

pub async fn scan(text: &str) -> (String, Vec<Entity<Text>>) {
    scan_scoped(text, Scope::new()).await
}

/// [`scan`] with the request scope asserting `country`, for tests that
/// exercise jurisdiction-gated behavior (e.g. a national-format phone
/// validator that needs a region to accept a number).
pub async fn scan_in_country(text: &str, country: &str) -> (String, Vec<Entity<Text>>) {
    let scope =
        Scope::new().with_country(country.parse::<CountryCode>().expect("valid ISO country"));
    scan_scoped(text, scope).await
}

async fn scan_scoped(text: &str, scope: Scope) -> (String, Vec<Entity<Text>>) {
    let recognizer = PatternRecognizer::builder()
        .with_builtin_patterns()
        .with_builtin_dictionaries()
        .with_builtin_walks()
        .build()
        .expect("shipped recognizer builds");
    let data = TextData::new(text.to_owned());
    let ctx = Context::new(&scope);
    let subject = Subject::new(data);
    let entities = recognizer
        .recognize(&subject, &ctx)
        .await
        .expect("shipped recognize");
    (text.to_owned(), entities)
}

/// Assert that some entity labeled `$label` is located at the exact substring
/// `$needle` in `$text` (presence by label *and* value). A macro, so a failure
/// is reported at the test call site. Text-specific: it reads the located byte
/// range off a [`TextLocation`], so it lives here rather than in the
/// modality-generic `elide_core` assertions.
#[macro_export]
macro_rules! assert_match {
    ($text:expr, $entities:expr, $label:expr, $needle:expr $(,)?) => {{
        let text: &str = $text;
        let entities = $entities;
        let label = $label;
        let needle: &str = $needle;
        let hit = entities.iter().any(|e| {
            e.label == label
                && e.location
                    .range()
                    .is_some_and(|r| text.get(r.start..r.end) == Some(needle))
        });
        assert!(
            hit,
            "expected `{needle}` as {label:?}; got: {:?}",
            entities
                .iter()
                .map(|e| {
                    let r = e.location.range().unwrap();
                    (e.label.clone(), &text[r.start..r.end])
                })
                .collect::<Vec<_>>()
        );
    }};
}
