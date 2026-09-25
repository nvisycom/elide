//! JSON Schema generation smoke tests for the image modality's `schema` feature.
//!
//! Proves the `JsonSchema` derives compile and produce a usable schema for the
//! image types, and that the generic `Entity<Image>` / `AuditLog<Image>` schema
//! names carry the modality prefix so they don't collide with another modality's
//! in a combined OpenAPI document.

#![cfg(feature = "schema")]

use elide_core::entity::Entity;
use elide_core::entity::audit::AuditLog;
use elide_core::modality::text::Text;
use elide_image::modality::{Image, ImageLocation};
use elide_image::primitive::{BoundingBox, Dimensions, Point, Polygon};
use schemars::{JsonSchema, schema_for};

/// The image location type generates a clean schema.
#[test]
fn image_modality_schema() {
    let _ = schema_for!(ImageLocation);
}

/// Generic types carry the modality in their schema name, so two modalities'
/// schemas do not collide into `AuditLog` / `AuditLog2` in a combined OpenAPI
/// document. See the `schemars(rename = "{M}...")` on these types.
#[test]
fn generic_schema_names_carry_modality_prefix() {
    assert_eq!(Entity::<Text>::schema_name(), "TextEntity");
    assert_eq!(Entity::<Image>::schema_name(), "ImageEntity");
    assert_eq!(AuditLog::<Text>::schema_name(), "TextAuditLog");
    assert_eq!(AuditLog::<Image>::schema_name(), "ImageAuditLog");
}

/// The generic geometry primitives carry the coordinate scalar in their schema
/// name (the same way [`Entity`] carries its modality), so `<u32>` and `<f64>`
/// do not collide into `BoundingBox` / `BoundingBox2`.
#[test]
fn geometry_schema_names_carry_coordinate_suffix() {
    assert_eq!(Point::<u32>::schema_name(), "PointU32");
    assert_eq!(Point::<f64>::schema_name(), "PointF64");
    assert_eq!(BoundingBox::<u32>::schema_name(), "BoundingBoxU32");
    assert_eq!(BoundingBox::<f64>::schema_name(), "BoundingBoxF64");
    assert_eq!(Dimensions::<u32>::schema_name(), "DimensionsU32");
    assert_eq!(Polygon::<f64>::schema_name(), "PolygonF64");
}

/// A document holding both coordinate variants of a primitive keeps them as
/// distinct, coordinate-consistent `$defs`, with no schemars auto-disambiguation
/// suffix and each box referencing its own coordinate's point.
#[test]
fn geometry_variants_share_defs_without_collision() {
    #[derive(JsonSchema)]
    #[allow(dead_code)]
    struct Both {
        a: BoundingBox<u32>,
        b: BoundingBox<f64>,
    }
    let json = serde_json::to_value(schema_for!(Both)).unwrap();
    let defs = json["$defs"].as_object().unwrap();
    assert!(defs.contains_key("BoundingBoxU32"));
    assert!(defs.contains_key("BoundingBoxF64"));
    assert!(defs.contains_key("PointU32"));
    assert!(defs.contains_key("PointF64"));
    assert!(!defs.contains_key("BoundingBox2"));
    assert!(!defs.contains_key("Point2"));
    assert_eq!(
        defs["BoundingBoxU32"]["properties"]["min"]["$ref"],
        "#/$defs/PointU32"
    );
    // Field doc-comments are preserved as property descriptions (as the derive
    // would have), alongside the `$ref` to the shared coordinate schema.
    assert_eq!(
        defs["BoundingBoxU32"]["properties"]["min"]["description"],
        "Minimum corner (top-left, conventionally)."
    );
    assert_eq!(
        defs["PointU32"]["properties"]["x"]["description"],
        "Horizontal coordinate."
    );
}
