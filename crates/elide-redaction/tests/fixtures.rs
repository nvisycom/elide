//! Shared fixtures for the `anonymize` integration tests. The in-memory
//! read/write [`TextDoc`] and the [`entity`] builders come from
//! `elide_core::test_util` (behind its `test-utils` feature); this module
//! re-exports them and adds a one-shot [`anonymize_one`] runner.
//! Compiled into each sibling test binary via `mod fixtures;`.

#![allow(dead_code, unused_imports)]

use elide_core::entity::Entity;
use elide_core::modality::text::Text;
use elide_core::recognition::Scope;
pub use elide_core::test_util::{TextDoc, entity, entity_conf};
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
