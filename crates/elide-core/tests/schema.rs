//! JSON Schema generation smoke tests for the `schema` feature.
//!
//! Proves the `JsonSchema` derives compile *and* produce a usable schema for
//! the tricky cases: a generic type (`Entity<M>` with its `bound`), fields of
//! external string-like types rendered via `schemars(with = ...)`, and a
//! feature-gated modality type.

#![cfg(feature = "schema")]

use elide_core::entity::Entity;
use elide_core::entity::audit::AuditEvent;
use elide_core::modality::text::{Text, TextLocation};
use elide_core::recognition::Scope;
use elide_core::recognition::annotation::Annotations;
use schemars::schema_for;

/// The generic `Entity<Text>` generates a schema covering its fields, with the
/// `LanguageTag` field rendered as a string (via `with = "Option<String>"`).
#[test]
fn entity_text_schema() {
    let schema = schema_for!(Entity<Text>);
    let json = serde_json::to_value(&schema).unwrap();
    let text = json.to_string();
    for field in ["id", "label", "location", "confidence", "language", "audit"] {
        assert!(text.contains(field), "schema should mention `{field}`");
    }
}

/// The location schema is a closed set of tagged shapes, not a bag of optional
/// fields: a `oneOf`/`anyOf` whose arms each require their own discriminant, so a
/// location naming nothing (`{}`) matches none of them. (This is the invariant
/// the whole enum redesign exists to express to downstream SDK consumers.)
#[test]
fn text_location_schema() {
    let schema = schema_for!(TextLocation);
    let json = serde_json::to_value(&schema).unwrap();
    let text = json.to_string();
    // The span range still appears (the detected arms carry it)...
    assert!(
        text.contains("start"),
        "schema must describe the span range"
    );
    // ...and the shape is a union of closed variants, keyed by `kind`.
    assert!(
        text.contains("oneOf") || text.contains("anyOf"),
        "location schema must be a union of variant shapes, got: {text}"
    );
    assert!(
        text.contains("kind"),
        "variant arms must carry a `kind` tag"
    );
}

/// The audit `AuditEvent<M>` (rich enum with external-string and bound cases)
/// generates without panicking.
#[test]
fn event_schema() {
    let _ = schema_for!(AuditEvent<Text>);
}

/// The caller-config `Scope` generates a schema covering its fields.
#[test]
fn scope_schema() {
    let schema = schema_for!(Scope);
    let json = serde_json::to_value(&schema).unwrap();
    let text = json.to_string();
    for field in ["languages", "countries", "labels", "catalog"] {
        assert!(
            text.contains(field),
            "scope schema should mention `{field}`"
        );
    }
}

/// The generic `Annotations<Text>` (Vec of M::Location-bounded regions)
/// generates without panicking.
#[test]
fn annotations_schema() {
    let _ = schema_for!(Annotations<Text>);
}

/// Generic types carry the modality in their schema name, so two modalities'
/// schemas do not collide into `AuditLog` / `AuditLog2` in a combined
/// OpenAPI document. See the `schemars(rename = "{M}...")` on these types.
/// (The cross-modality collision is exercised with a non-core modality in that
/// modality crate's own tests, e.g. `elide-image`.)
#[test]
fn generic_schema_name_carries_modality_prefix() {
    use elide_core::entity::audit::AuditLog;
    use schemars::JsonSchema;

    assert_eq!(Entity::<Text>::schema_name(), "TextEntity");
    assert_eq!(AuditLog::<Text>::schema_name(), "TextAuditLog");
}
