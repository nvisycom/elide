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
