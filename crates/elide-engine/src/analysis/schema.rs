//! Hand-written [`JsonSchema`] for [`Report`] and [`ArtifactSet`], kept in
//! lockstep with their [`Serialize`](super::serde) impls so a generated schema
//! never drifts from the wire form.
//!
//! `Report` serializes through a hand-written `Serialize` (its groups are
//! type-erased), so schemars cannot derive this. The one open piece is a group's
//! `entities`, whose element type depends on the sibling `modality` string — a
//! discriminated union, expressed as `oneOf` over per-modality group arms with a
//! `discriminator` on `modality`. Each arm is gated on its modality feature, so
//! the schema enumerates exactly the modalities compiled in (`text` always).
//!
//! A round-trip test (in [`serde`](super::serde)'s test module)
//! validates a serialized report against this schema, turning any drift into a
//! failing test rather than a lying client.
//!
//! Both types share the `{ body, parts }` shape and the same erasure problem,
//! so both are expressed the same way — a `oneOf` over per-modality arms. They
//! differ only in what an arm carries: a `Report` arm holds `entities`, an
//! `ArtifactSet` arm holds the single `artifact` for that modality.
//!
//! [`Report`]: super::report::Report
//! [`ArtifactSet`]: super::artifacts::ArtifactSet

use std::borrow::Cow;

use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::modality::text::Text;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};

use super::artifacts::ArtifactSet;
use super::report::Report;

/// The `{ modality: "<name>", entities: [Entity<M>] }` arm for one modality,
/// with `modality` pinned to a `const` so it discriminates the union.
fn group_arm<M: Modality>(generator: &mut SchemaGenerator) -> Schema
where
    Entity<M>: JsonSchema,
{
    let entity = generator.subschema_for::<Entity<M>>();
    json_schema!({
        "type": "object",
        "properties": {
            "modality": { "type": "string", "const": M::NAME },
            "entities": { "type": "array", "items": entity },
        },
        "required": ["modality", "entities"],
    })
}

impl JsonSchema for Report {
    fn schema_name() -> Cow<'static, str> {
        "Report".into()
    }

    fn schema_id() -> Cow<'static, str> {
        concat!(module_path!(), "::Report").into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        // A group is one of the per-modality arms, discriminated by `modality`.
        // Only the compiled-in modalities appear.
        let arms = vec![
            group_arm::<Text>(generator),
            #[cfg(feature = "image")]
            group_arm::<elide_core::modality::image::Image>(generator),
            #[cfg(feature = "audio")]
            group_arm::<elide_core::modality::audio::Audio>(generator),
            #[cfg(feature = "tabular")]
            group_arm::<elide_core::modality::tabular::Tabular>(generator),
        ];
        let group = json_schema!({
            "oneOf": arms,
            "discriminator": { "propertyName": "modality" },
        });

        // `body` is a group or null; `parts` maps a part id to a group. `mut` is
        // used only to splice in `usage` under that feature.
        #[cfg_attr(not(feature = "usage"), allow(unused_mut))]
        let mut schema = json_schema!({
            "type": "object",
            "properties": {
                "body": { "oneOf": [group.clone(), { "type": "null" }] },
                "parts": { "type": "object", "additionalProperties": group },
            },
            "required": ["body", "parts"],
        });

        // `usage` is present only under the `usage` feature — mirror the
        // conditional field in `Serialize`.
        #[cfg(feature = "usage")]
        {
            let usage = generator.subschema_for::<elide_core::recognition::UsageReport>();
            let props = schema
                .ensure_object()
                .entry("properties")
                .or_insert_with(|| serde_json::Value::Object(Default::default()));
            if let Some(props) = props.as_object_mut() {
                props.insert("usage".into(), usage.to_value());
            }
        }

        schema
    }
}

/// The `{ modality: "<name>", artifact: <M::Artifact> }` arm for one modality,
/// with `modality` pinned to a `const` so it discriminates the union.
///
/// The artifact-side counterpart to [`group_arm`]: same discriminated shape,
/// carrying one modality's enrichment (an image's `Layout`, an audio clip's
/// `Transcription`) instead of its entities.
fn artifact_arm<M: Modality>(generator: &mut SchemaGenerator) -> Schema
where
    M::Artifact: JsonSchema,
{
    let artifact = generator.subschema_for::<M::Artifact>();
    json_schema!({
        "type": "object",
        "properties": {
            "modality": { "type": "string", "const": M::NAME },
            "artifact": artifact,
        },
        "required": ["modality", "artifact"],
    })
}

impl JsonSchema for ArtifactSet {
    fn schema_name() -> Cow<'static, str> {
        "ArtifactSet".into()
    }

    fn schema_id() -> Cow<'static, str> {
        concat!(module_path!(), "::ArtifactSet").into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        // One arm per compiled-in modality, discriminated by `modality` —
        // mirroring `Report`, so a client reads both halves the same way.
        let arms = vec![
            artifact_arm::<Text>(generator),
            #[cfg(feature = "image")]
            artifact_arm::<elide_core::modality::image::Image>(generator),
            #[cfg(feature = "audio")]
            artifact_arm::<elide_core::modality::audio::Audio>(generator),
            #[cfg(feature = "tabular")]
            artifact_arm::<elide_core::modality::tabular::Tabular>(generator),
        ];
        let group = json_schema!({
            "oneOf": arms,
            "discriminator": { "propertyName": "modality" },
        });

        // `body` is a group or null; `parts` maps a part id to a group. No
        // `usage` here — that rides on the report, not the enrichment.
        json_schema!({
            "type": "object",
            "properties": {
                "body": { "oneOf": [group.clone(), { "type": "null" }] },
                "parts": { "type": "object", "additionalProperties": group },
            },
            "required": ["body", "parts"],
        })
    }
}
