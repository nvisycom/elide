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
/// fields: a union (`anyOf`) of `Detected`/`Marked`, each a `oneOf` of
/// `kind`-tagged arms that list their discriminant under `required`. A location
/// naming nothing (`{}`) matches no arm. This is the invariant the whole enum
/// redesign exists to express to downstream SDK consumers, so the test asserts
/// the structure (not substrings, which could match a stray description word).
#[test]
fn text_location_schema() {
    let schema = schema_for!(TextLocation);
    let json = serde_json::to_value(&schema).unwrap();

    // Top level is a union of the two sub-schemas.
    let arms = json
        .get("anyOf")
        .or_else(|| json.get("oneOf"))
        .and_then(|v| v.as_array())
        .expect("location schema is a union of Detected/Marked");
    assert_eq!(arms.len(), 2, "two top-level arms (Detected, Marked)");

    // Every leaf variant (resolving $refs into $defs) is a closed object that
    // lists `kind` under `required` — so none of them accepts `{}`.
    let defs = json.get("$defs").and_then(|v| v.as_object());
    let leaf_arms = |node: &serde_json::Value| -> Vec<serde_json::Value> {
        let resolved = node
            .get("$ref")
            .and_then(|r| r.as_str())
            .and_then(|r| r.strip_prefix("#/$defs/"))
            .and_then(|name| defs.and_then(|d| d.get(name)))
            .cloned()
            .unwrap_or_else(|| node.clone());
        resolved
            .get("oneOf")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_else(|| vec![resolved])
    };
    let mut leaves = 0;
    let mut anchor_arrays = 0;
    for arm in arms {
        for leaf in leaf_arms(arm) {
            leaves += 1;
            let required = leaf
                .get("required")
                .and_then(|v| v.as_array())
                .expect("each variant lists required fields");
            assert!(
                required.iter().any(|f| f == "kind"),
                "variant arm must require `kind`: {leaf}"
            );
            // An anchor array (`refs`/`rects`) must carry `minItems: 1` — the
            // non-empty invariant expressed in the schema, not just at deserialize,
            // so a generated SDK validator rejects an empty anchor too.
            for field in ["refs", "rects"] {
                if let Some(array) = leaf.get("properties").and_then(|p| p.get(field)) {
                    anchor_arrays += 1;
                    assert_eq!(
                        array.get("minItems").and_then(|m| m.as_u64()),
                        Some(1),
                        "`{field}` must require minItems: 1: {leaf}"
                    );
                }
            }
        }
    }
    // Inline, Source, Rendered (detected) + Source, Rendered (marked) = 5.
    assert_eq!(leaves, 5, "five concrete variant shapes");
    // Source + Rendered on each of Detected and Marked = 4 anchor arrays.
    assert_eq!(anchor_arrays, 4, "four non-empty anchor arrays");
}

/// The authoritative runtime contract behind the schema: the serde impl rejects
/// a location that names nothing and round-trips every real variant. A generated
/// SDK validator mirrors the schema, but this is the behavior the schema
/// describes, tested directly with no extra dependency.
#[cfg(feature = "serde")]
#[test]
fn text_location_rejects_empty_and_round_trips() {
    use elide_core::modality::text::SourceRef;
    use elide_core::primitive::UnitBoundingBox;

    // A location naming nothing is unrepresentable on the wire.
    for bad in [
        "{}",
        r#"{"kind":"source","span":{"start":0,"end":5},"refs":[]}"#,
        r#"{"kind":"source","refs":[]}"#,
        r#"{"kind":"rendered","span":{"start":0,"end":5},"page":1,"rects":[]}"#,
    ] {
        assert!(
            serde_json::from_str::<TextLocation>(bad).is_err(),
            "must reject a location that names nothing: {bad}"
        );
    }

    // Every real variant round-trips.
    let full = UnitBoundingBox::new(0.0, 0.0, 1.0, 1.0);
    for good in [
        TextLocation::inline(0..5),
        TextLocation::source(0..5, [SourceRef::new(2..4)]),
        TextLocation::rendered(0..5, 3, [full]),
        TextLocation::marked_source(SourceRef::new(3..12)),
        TextLocation::marked_rendered(2, full),
    ] {
        let json = serde_json::to_string(&good).expect("serializes");
        let back: TextLocation = serde_json::from_str(&json).expect("round-trips");
        assert_eq!(back, good, "round-trip mismatch for {json}");
    }
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
