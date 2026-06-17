//! Shared helpers for the `builtin_*` end-to-end test suites.
//!
//! Each per-region test file (`tests/builtin.rs`,
//! `tests/builtin_us.rs`, `tests/builtin_uk.rs`, …) declares this
//! module via `mod fixtures;` and calls [`scan`] + the
//! `assert_*` helpers to express expectations against a single
//! shared [`PatternRecognizer`] built from every shipped pattern
//! and dictionary.

use veil_core::entity::{Entity, LabelRef};
use veil_core::modality::text::{Text, TextData};
use veil_core::recognition::{Recognizer, RecognizerInput};
use veil_pattern::PatternRecognizer;

pub async fn scan(text: &str) -> (String, Vec<Entity<Text>>) {
    let recognizer = PatternRecognizer::builder()
        .with_builtin_patterns()
        .with_builtin_dictionaries()
        .build_context_enhanced()
        .expect("shipped recognizer builds");
    let input = RecognizerInput::new(TextData::new(text.to_owned()));
    let entities = recognizer
        .recognize(&input)
        .await
        .expect("shipped recognize")
        .entities;
    (text.to_owned(), entities)
}

#[track_caller]
pub fn assert_match(text: &str, entities: &[Entity<Text>], label: LabelRef, needle: &str) {
    let hit = entities
        .iter()
        .any(|e| e.label == label && &text[e.location.start..e.location.end] == needle);
    assert!(
        hit,
        "expected `{needle}` as {label:?}; got: {:?}",
        entities
            .iter()
            .map(|e| (e.label.clone(), &text[e.location.start..e.location.end]))
            .collect::<Vec<_>>()
    );
}

#[track_caller]
pub fn assert_label_present(entities: &[Entity<Text>], label: LabelRef) {
    assert!(
        entities.iter().any(|e| e.label == label),
        "expected at least one {label:?} entity; got labels: {:?}",
        entities.iter().map(|e| e.label.clone()).collect::<Vec<_>>()
    );
}
