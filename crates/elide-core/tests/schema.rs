//! JSON Schema generation smoke tests for the `schema` feature.
//!
//! Proves the `JsonSchema` derives compile *and* produce a usable schema for
//! the tricky cases: a generic type (`Entity<M>` with its `bound`), fields of
//! external string-like types rendered via `schemars(with = ...)`, and a
//! feature-gated modality type.

#![cfg(feature = "schema")]

use elide_core::entity::Entity;
use elide_core::entity::provenance::Event;
use elide_core::modality::text::{Text, TextLocation};
use schemars::schema_for;

/// The generic `Entity<Text>` generates a schema covering its fields, with the
/// `LanguageTag` field rendered as a string (via `with = "Option<String>"`).
#[test]
fn entity_text_schema() {
    let schema = schema_for!(Entity<Text>);
    let json = serde_json::to_value(&schema).unwrap();
    let text = json.to_string();
    for field in ["id", "label", "location", "confidence", "language", "provenance"] {
        assert!(text.contains(field), "schema should mention `{field}`");
    }
}

/// A plain location type generates a clean object schema.
#[test]
fn text_location_schema() {
    let schema = schema_for!(TextLocation);
    let json = serde_json::to_value(&schema).unwrap();
    assert!(json.to_string().contains("start"));
}

/// The provenance `Event<M>` (rich enum with external-string and bound cases)
/// generates without panicking.
#[test]
fn event_schema() {
    let _ = schema_for!(Event<Text>);
}

#[cfg(feature = "image")]
#[test]
fn image_modality_schema() {
    use elide_core::modality::image::ImageLocation;
    let _ = schema_for!(ImageLocation);
}
