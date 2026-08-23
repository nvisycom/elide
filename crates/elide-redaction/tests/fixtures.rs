//! Shared fixtures for the `anonymize` integration tests. Re-exports the
//! in-memory read/write [`TextDoc`] (and [`Text`]) from `elide_core` — behind
//! its `test-util` feature — and adds a one-shot [`anonymize_one`] runner.
//! Entity fixtures come straight from [`Entity::fixture`], which callers use
//! directly. Compiled into each sibling test binary via `mod fixtures;`.
//!
//! [`Entity::fixture`]: elide_core::entity::Entity::fixture

#![allow(dead_code, unused_imports)]

use elide_core::entity::Entity;
pub use elide_core::modality::text::{Text, TextDoc};
use elide_core::recognition::Scope;
use elide_redaction::Anonymizer;

/// Run one anonymizer over `source` for a single entity and return the written
/// document text.
pub async fn anonymize_one(anonymizer: Anonymizer<Text>, source: &str, e: Entity<Text>) -> String {
    let mut doc = TextDoc::new(source);
    let mut entities = vec![e];
    anonymizer
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();
    doc.text().to_owned()
}
