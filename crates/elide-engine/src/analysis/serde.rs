//! The serde wire form for the analysis views: the [`Report`]'s
//! `serde::Serialize` and the seed visitors that reconstruct both a [`Report`]
//! and an [`ArtifactSet`] from their `{ body, parts }` wire shape.
//!
//! The serialized report is the part-grouped `{ body, parts }` view a review
//! layer consumes; each group is tagged with its
//! [`modality`](elide_core::modality::Modality::NAME) so the orchestrator can
//! route it back on deserialization. Reconstruction is driven by the
//! [`ModalityRegistry`](super::registry::ModalityRegistry): the wire form drops
//! the concrete modality type, deserialization is not object-safe, so each group
//! is buffered and replayed through the per-modality parser resolved from the
//! registry.
//!
//! Both views share the same wire shape — `{ body, parts }` of `{ modality, X }`
//! groups — so one generic seed family drives both, parameterized by a [`Leaf`]:
//! the entities of a [`Report`] or the artifact of an [`ArtifactSet`]. The two
//! differ only at the leaf: the field name (`entities` / `artifact`), which
//! registry parser to run, and what an *unregistered* modality means. For
//! entities, an unregistered group is skipped when empty (nothing to lose,
//! matching how `analyze` ignores an unmatched part) but a hard error when
//! non-empty — its entities may carry reviewer edits that silently dropping the
//! group would lose, and an absent/null `entities` field is likewise rejected.
//! An artifact carries no reviewer-editable state, so anything that reconstructs
//! to nothing — an unregistered modality, or an absent/null `artifact` field —
//! is simply dropped, and a re-run re-enriches.
//!
//! [`Report`]: super::report::Report
//! [`ArtifactSet`]: super::artifacts::ArtifactSet

use std::any::TypeId;
use std::collections::HashMap;
use std::marker::PhantomData;

use serde::de::{
    DeserializeSeed, Deserializer, Error as DeError, IntoDeserializer, MapAccess, Visitor,
};
use serde_value::Value;

use super::artifacts::ArtifactSet;
use super::group::{ArtifactGroup, EntityGroup};
use super::registry::{ModalityEntry, ModalityRegistry};
use super::report::{BodyReport, PartReport, Report};
use crate::PartId;

impl serde::Serialize for Report {
    /// Serialize to `{ body: {modality, entities}, parts: { id: {modality,
    /// entities} } }`. `body` is null when no body pipeline ran. Each group
    /// carries its modality name so it can be parsed back into the right
    /// `Vec<Entity<M>>`.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        // Adapt an erased group to a Serialize value: `{ modality, entities }`.
        struct Group<'a>(&'a dyn EntityGroup);
        impl serde::Serialize for Group<'_> {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                // The entities serialize through erasure; `modality` tags which
                // `M` to parse them back as.
                struct Entities<'a>(&'a dyn EntityGroup);
                impl serde::Serialize for Entities<'_> {
                    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                        erased_serde::serialize(self.0, s)
                    }
                }
                let mut state = s.serialize_struct("Group", 2)?;
                state.serialize_field("modality", self.0.modality_name())?;
                state.serialize_field("entities", &Entities(self.0))?;
                state.end()
            }
        }

        let parts: HashMap<&str, Group<'_>> = self
            .parts
            .iter()
            .map(|(id, p)| (id.as_str(), Group(p.entities.as_ref())))
            .collect();

        // `usage` is the third field only under the `usage` feature.
        #[cfg(feature = "usage")]
        let field_count = 3;
        #[cfg(not(feature = "usage"))]
        let field_count = 2;

        let mut state = serializer.serialize_struct("Report", field_count)?;
        state.serialize_field(
            "body",
            &self.body.as_ref().map(|b| Group(b.entities.as_ref())),
        )?;
        state.serialize_field("parts", &parts)?;
        #[cfg(feature = "usage")]
        state.serialize_field("usage", &self.usage)?;
        state.end()
    }
}

// ---- The generic deserialization core -------------------------------------
//
// Both a `Report` (entities) and an `ArtifactSet` (artifacts) deserialize from
// the same `{ body, parts }` wire shape of `{ modality, X }` groups. A `Leaf`
// captures everything that differs between the two: the field name, the struct
// names, which registry parser reconstructs a buffered group, and how a parsed
// group is stored into the final set. The seeds below are generic over it.

/// What the two views (a [`Report`]'s entities, an [`ArtifactSet`]'s artifacts)
/// differ by. Everything else — the buffered, order-independent `{ modality, X }`
/// traversal and the `{ body, parts }` assembly — is shared.
pub(super) trait Leaf {
    /// One reconstructed group of this leaf: the concrete boxed value plus the
    /// routing metadata the set entry keys on.
    type Parsed;
    /// The set this leaf reconstructs: [`Report`] or [`ArtifactSet`].
    type Set;

    /// The wire field carrying this leaf's payload inside a group envelope:
    /// `"entities"` for a report, `"artifact"` for an artifact set.
    const FIELD: &'static str;
    /// The group envelope's struct name: `"Group"` / `"ArtifactGroup"`.
    const GROUP_NAME: &'static str;
    /// The whole-set struct name: `"Report"` / `"ArtifactSet"`.
    const SET_NAME: &'static str;
    /// What `expecting` writes for one group.
    const GROUP_EXPECTING: &'static str;
    /// What `expecting` writes for the whole set.
    const SET_EXPECTING: &'static str;

    /// Reconstruct one group's payload, applying this leaf's unregistered-modality
    /// policy. `name` is the group's modality tag and `value` its buffered
    /// payload (already read from the `FIELD` field). Returns `None` when the
    /// group is skipped.
    fn parse<E: DeError>(
        entry: Option<ModalityEntry>,
        name: &str,
        value: Value,
    ) -> Result<Option<Self::Parsed>, E>;

    /// An empty set to fill in.
    fn empty() -> Self::Set;
    /// Store a reconstructed body group into the set.
    fn set_body(set: &mut Self::Set, parsed: Self::Parsed);
    /// Store a reconstructed part group, keyed by `id`, into the set.
    fn set_part(set: &mut Self::Set, id: PartId, parsed: Self::Parsed);
}

/// The report leaf: a group's `entities` reconstruct as a boxed
/// `Vec<Entity<M>>`. An unregistered modality is skipped only when empty; a
/// non-empty one is a hard error, since its entities may carry reviewer edits.
pub(super) struct EntityLeaf;

/// One reconstructed entity group: the entities and the routing [`TypeId`] the
/// report entry keys on (matching [`BodyReport::modality`] /
/// [`PartReport::modality`]).
pub(super) struct ParsedGroup {
    modality: TypeId,
    entities: Box<dyn EntityGroup>,
}

impl Leaf for EntityLeaf {
    type Parsed = ParsedGroup;
    type Set = Report;

    const FIELD: &'static str = "entities";
    const GROUP_EXPECTING: &'static str = "a { modality, entities } group";
    const GROUP_NAME: &'static str = "Group";
    const SET_EXPECTING: &'static str = "a { body, parts } report";
    const SET_NAME: &'static str = "Report";

    fn parse<E: DeError>(
        entry: Option<ModalityEntry>,
        name: &str,
        value: Value,
    ) -> Result<Option<ParsedGroup>, E> {
        // Entities are required — an absent/null `entities` field (`Value::Unit`)
        // is malformed, not "no entities": unlike an artifact, a missing entity
        // group could be silently dropping a reviewer's edits.
        if matches!(value, Value::Unit) {
            return Err(E::missing_field("entities"));
        }
        let Some(entry) = entry else {
            // Unregistered modality: skip only an empty group (nothing to lose,
            // as in `analyze`); reject a non-empty one, whose entities a reviewer
            // may have edited.
            if is_empty_entities(&value) {
                return Ok(None);
            }
            return Err(E::custom(format!(
                "no registered modality for `{name}` (its {} entities would be dropped)",
                entity_count(&value),
            )));
        };

        // Replay the buffered entities through the modality's parser.
        let mut erased = <dyn erased_serde::Deserializer<'_>>::erase(value.into_deserializer());
        let group = (entry.parse)(&mut erased).map_err(E::custom)?;
        Ok(Some(ParsedGroup {
            modality: entry.type_id,
            entities: group,
        }))
    }

    fn empty() -> Report {
        Report::new()
    }

    fn set_body(report: &mut Report, parsed: ParsedGroup) {
        report.body = Some(BodyReport {
            modality: parsed.modality,
            entities: parsed.entities,
        });
    }

    fn set_part(report: &mut Report, id: PartId, parsed: ParsedGroup) {
        report.parts.insert(
            id,
            PartReport {
                modality: parsed.modality,
                handle: None,
                entities: parsed.entities,
            },
        );
    }
}

/// Whether a buffered `entities` value is an empty list — the group carries no
/// entities, so skipping it (for an unregistered modality) loses nothing.
fn is_empty_entities(entities: &Value) -> bool {
    entity_count(entities) == 0
}

/// The number of entities a buffered `entities` value carries. The serializer
/// emits an array; anything else counts as non-empty so it is never silently
/// dropped.
fn entity_count(entities: &Value) -> usize {
    match entities {
        Value::Seq(items) => items.len(),
        _ => usize::MAX,
    }
}

/// The artifact leaf: a group's `artifact` reconstructs as a boxed
/// `M::Artifact`. An unregistered *or* absent modality simply yields nothing —
/// an artifact carries no reviewer-editable state, so dropping it loses no work
/// (a re-run re-enriches).
pub(super) struct ArtifactLeaf;

/// One reconstructed artifact group: the erased artifact plus its routing
/// [`TypeId`] and modality name.
pub(super) struct ParsedArtifact {
    modality: TypeId,
    modality_name: &'static str,
    artifact: Box<dyn ArtifactGroup>,
}

impl Leaf for ArtifactLeaf {
    type Parsed = ParsedArtifact;
    type Set = ArtifactSet;

    const FIELD: &'static str = "artifact";
    const GROUP_EXPECTING: &'static str = "a { modality, artifact } group";
    const GROUP_NAME: &'static str = "ArtifactGroup";
    const SET_EXPECTING: &'static str = "a { body, parts } artifact set";
    const SET_NAME: &'static str = "ArtifactSet";

    fn parse<E: DeError>(
        entry: Option<ModalityEntry>,
        _name: &str,
        value: Value,
    ) -> Result<Option<ParsedArtifact>, E> {
        // A dropped artifact loses no work — a re-run re-enriches — so anything
        // that leaves nothing to reconstruct is skipped rather than rejected: an
        // unregistered modality (no parser), or an absent/null `artifact` field
        // (`Value::Unit`, a payload that was never enriched).
        if matches!(value, Value::Unit) {
            return Ok(None);
        }
        let Some(entry) = entry else {
            return Ok(None);
        };
        let mut erased = <dyn erased_serde::Deserializer<'_>>::erase(value.into_deserializer());
        let parsed = (entry.parse_artifact)(&mut erased).map_err(E::custom)?;
        Ok(Some(ParsedArtifact {
            modality: entry.type_id,
            modality_name: entry.modality_name,
            artifact: parsed,
        }))
    }

    fn empty() -> ArtifactSet {
        ArtifactSet::new()
    }

    fn set_body(set: &mut ArtifactSet, parsed: ParsedArtifact) {
        set.set_body(parsed.modality, parsed.modality_name, parsed.artifact);
    }

    fn set_part(set: &mut ArtifactSet, id: PartId, parsed: ParsedArtifact) {
        set.set_part(id, parsed.modality, parsed.modality_name, parsed.artifact);
    }
}

/// A serialized group envelope: `{ modality, X }` (`X` is `entities` or
/// `artifact`). Deserializes by buffering both fields (in any order — a review
/// layer may reorder keys), resolving the `modality`'s registered entry, then
/// running that entry's parser over the buffered payload. Yields `None` when the
/// leaf's unregistered-modality policy skips the group.
struct GroupSeed<'a, L> {
    registry: &'a ModalityRegistry,
    _leaf: PhantomData<L>,
}

impl<'a, L> GroupSeed<'a, L> {
    fn new(registry: &'a ModalityRegistry) -> Self {
        Self {
            registry,
            _leaf: PhantomData,
        }
    }
}

impl<'de, L: Leaf> DeserializeSeed<'de> for GroupSeed<'_, L> {
    type Value = Option<L::Parsed>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_struct(L::GROUP_NAME, &["modality", L::FIELD], self)
    }
}

impl<'de, L: Leaf> Visitor<'de> for GroupSeed<'_, L> {
    type Value = Option<L::Parsed>;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(L::GROUP_EXPECTING)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        // Order-independent: a review layer's JSON tooling may reorder keys, so
        // the payload can arrive before `modality`. Buffer both, then resolve the
        // parser and parse the payload after the map is fully read. Unknown
        // fields are ignored (as at the set level) so the format can grow.
        let mut modality: Option<String> = None;
        let mut payload: Option<Value> = None;
        while let Some(key) = map.next_key::<String>()? {
            if key == "modality" {
                if modality.is_some() {
                    return Err(DeError::duplicate_field("modality"));
                }
                modality = Some(map.next_value()?);
            } else if key == L::FIELD {
                if payload.is_some() {
                    return Err(DeError::duplicate_field(L::FIELD));
                }
                // Buffered, not parsed yet: the modality (hence the parser) may
                // not have been seen. A `Value` round-trips into a deserializer
                // in `Leaf::parse`.
                payload = Some(map.next_value()?);
            } else {
                map.next_value::<serde::de::IgnoredAny>()?;
            }
        }
        let name = modality.ok_or_else(|| DeError::missing_field("modality"))?;
        // An absent payload field is `Value::Unit` (the same value a JSON `null`
        // yields), so `Leaf::parse` decides what a missing payload means for its
        // side — entities require one, an artifact treats it as "not enriched".
        let payload = payload.unwrap_or(Value::Unit);
        L::parse(self.registry.entry(&name), &name, payload)
    }
}

/// `body` is `Option<group>`: null when this side has no body.
struct OptionGroupSeed<'a, L> {
    registry: &'a ModalityRegistry,
    _leaf: PhantomData<L>,
}

impl<'de, L: Leaf> DeserializeSeed<'de> for OptionGroupSeed<'_, L> {
    type Value = Option<L::Parsed>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_option(self)
    }
}

impl<'de, L: Leaf> Visitor<'de> for OptionGroupSeed<'_, L> {
    type Value = Option<L::Parsed>;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("null or a group")
    }

    fn visit_none<E: DeError>(self) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_unit<E: DeError>(self) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        // `GroupSeed` already yields `Option`: a skipped unregistered-and-empty
        // body flattens to `None`, exactly like a null body.
        GroupSeed::<L>::new(self.registry).deserialize(d)
    }
}

/// `parts` is a map of `PartId` -> group.
struct PartsSeed<'a, L> {
    registry: &'a ModalityRegistry,
    _leaf: PhantomData<L>,
}

impl<'de, L: Leaf> DeserializeSeed<'de> for PartsSeed<'_, L> {
    type Value = HashMap<String, Option<L::Parsed>>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de, L: Leaf> Visitor<'de> for PartsSeed<'_, L> {
    type Value = HashMap<String, Option<L::Parsed>>;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a map of part id to group")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut out = HashMap::new();
        while let Some(id) = map.next_key::<String>()? {
            // A repeated part id would silently overwrite the earlier group — and
            // its entity edits — so reject it. Checked on the id (before the
            // value) so a duplicate is caught even when one side is skipped.
            if out.contains_key(&id) {
                return Err(DeError::custom(format!("duplicate part id `{id}`")));
            }
            // A part whose modality is unregistered and empty is skipped (`None`),
            // matching how `analyze` ignores an unmatched part; a non-empty one
            // has already errored inside `GroupSeed`. A skipped part still
            // reserves its id (an empty entry) so a later duplicate is caught.
            let group = map.next_value_seed(GroupSeed::<L>::new(self.registry))?;
            out.insert(id, group);
        }
        Ok(out)
    }
}

/// The whole-set seed: `{ body, parts }`, each group parsed through the registry.
/// Generic over the [`Leaf`]; the [`ReportSeed`] / [`ArtifactSetSeed`] aliases
/// pin it to the two views. Drives [`Orchestrator::deserialize_report`] and
/// [`Orchestrator::deserialize_artifacts`].
///
/// [`Orchestrator::deserialize_report`]: crate::Orchestrator::deserialize_report
/// [`Orchestrator::deserialize_artifacts`]: crate::Orchestrator::deserialize_artifacts
pub(super) struct SetSeed<'a, L> {
    pub(super) registry: &'a ModalityRegistry,
    pub(super) _leaf: PhantomData<L>,
}

impl<'a, L> SetSeed<'a, L> {
    pub(super) fn new(registry: &'a ModalityRegistry) -> Self {
        Self {
            registry,
            _leaf: PhantomData,
        }
    }
}

/// Reconstructs a [`Report`] from its `{ body, parts }` wire form.
pub(super) type ReportSeed<'a> = SetSeed<'a, EntityLeaf>;
/// Reconstructs an [`ArtifactSet`] from its `{ body, parts }` wire form.
pub(super) type ArtifactSetSeed<'a> = SetSeed<'a, ArtifactLeaf>;

impl<'de, L: Leaf> serde::de::DeserializeSeed<'de> for SetSeed<'_, L> {
    type Value = L::Set;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<L::Set, D::Error> {
        deserializer.deserialize_struct(L::SET_NAME, &["body", "parts"], self)
    }
}

impl<'de, L: Leaf> Visitor<'de> for SetSeed<'_, L> {
    type Value = L::Set;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(L::SET_EXPECTING)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<L::Set, A::Error> {
        let mut set = L::empty();
        let mut seen_body = false;
        let mut seen_parts = false;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "body" => {
                    if seen_body {
                        return Err(DeError::duplicate_field("body"));
                    }
                    seen_body = true;
                    if let Some(parsed) = map.next_value_seed(OptionGroupSeed::<L> {
                        registry: self.registry,
                        _leaf: PhantomData,
                    })? {
                        L::set_body(&mut set, parsed);
                    }
                }
                "parts" => {
                    if seen_parts {
                        return Err(DeError::duplicate_field("parts"));
                    }
                    seen_parts = true;
                    let parts = map.next_value_seed(PartsSeed::<L> {
                        registry: self.registry,
                        _leaf: PhantomData,
                    })?;
                    // A `None` value is a part that was skipped (unregistered and
                    // empty); its id was reserved only to catch duplicates.
                    for (id, parsed) in parts {
                        if let Some(parsed) = parsed {
                            L::set_part(&mut set, PartId::from(id), parsed);
                        }
                    }
                }
                // `usage` (and any future field) is ignored: it is derived
                // analysis output, not editable review state. Both `body` and
                // `parts` are optional — a body-less document, or one with no
                // container parts, is a valid set.
                _ => {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
        }
        Ok(set)
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::audit::{AuditEvent, AuditLog, PatternEvent};
    use elide_core::entity::{Entity, LabelRef};
    use elide_core::modality::text::{Text, TextLocation};
    use elide_core::primitive::Confidence;
    use serde::de::DeserializeSeed;

    use super::*;
    use crate::PartId;

    fn text_entity(label: &str) -> Entity<Text> {
        let loc = TextLocation::new(0, 4);
        let event = AuditEvent::pattern("t", Confidence::MAX, loc.clone(), PatternEvent::default());
        Entity::new(
            LabelRef::new(label),
            loc,
            Confidence::MAX,
            AuditLog::new(event),
        )
    }

    /// A registry with `Text` registered — the unit under test needs only the
    /// group parsers, not a full orchestrator.
    fn text_registry() -> ModalityRegistry {
        let mut r = ModalityRegistry::default();
        r.register::<Text>();
        r
    }

    fn round_trip(report: &Report, registry: &ModalityRegistry) -> Report {
        let json = serde_json::to_string(report).expect("serialize");
        let mut de = serde_json::Deserializer::from_str(&json);
        match ReportSeed::new(registry).deserialize(&mut de) {
            Ok(report) => report,
            Err(e) => panic!("deserialize: {e}"),
        }
    }

    #[test]
    fn serializes_body_to_grouped_view() {
        // The part-grouped `{ body, parts }` shape is exercised end to end
        // (with a real container) in the docx integration test; here we
        // check the body group and the empty-parts shape directly.
        let report = Report::new().insert_body::<Text>(vec![text_entity("EMAIL_ADDRESS")]);

        let value = serde_json::to_value(&report).unwrap();
        // body is a `{ modality, entities }` group; parts is an object.
        assert_eq!(value["body"]["modality"], "text");
        assert_eq!(value["body"]["entities"][0]["label"], "EMAIL_ADDRESS");
        assert!(value["parts"].is_object());
        assert_eq!(value["parts"].as_object().unwrap().len(), 0);

        // No body pipeline ran → body is null.
        let empty = serde_json::to_value(Report::new()).unwrap();
        assert!(empty["body"].is_null());
    }

    /// The hand-written [`JsonSchema`](super::super::schema) must accept what
    /// [`Serialize`] produces — the drift guard. A populated report (body plus a
    /// container part), a body-less report, and an empty report all validate
    /// against `schema_for!(Report)`; any divergence between the two hand-written
    /// impls fails here rather than silently shipping a schema that lies.
    #[cfg(feature = "schema")]
    #[test]
    fn serialized_reports_validate_against_the_schema() {
        use elide_codec::PartId;

        let schema = serde_json::to_value(schemars::schema_for!(Report)).unwrap();

        let with_part = Report::new()
            .insert_body::<Text>(vec![text_entity("EMAIL_ADDRESS")])
            .insert_part::<Text>(
                PartId::from("word/media/image1.png".to_owned()),
                vec![text_entity("PHONE_NUMBER")],
            );

        for report in [
            with_part,
            Report::new().insert_body::<Text>(Vec::new()),
            Report::new(),
        ] {
            let doc = serde_json::to_value(&report).unwrap();
            if let Err(e) = jsonschema::validate(&schema, &doc) {
                panic!("serialized report does not match its schema: {e}\n{doc:#}");
            }
        }
    }

    #[cfg(feature = "usage")]
    #[test]
    fn serializes_usage_entries() {
        use std::time::Duration;

        use elide_core::recognition::{RecognizerId, Usage};

        let mut report = Report::new();
        report.usage.extend([Usage::new(
            RecognizerId::new("elide-pattern", "1"),
            Duration::from_millis(5),
            3,
        )]);

        let value = serde_json::to_value(&report).unwrap();
        let entries = value["usage"]["entries"].as_array().expect("usage array");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["id"]["name"], "elide-pattern");
        assert_eq!(entries[0]["duration"], 5);
        assert_eq!(entries[0]["count"], 3);
        // An empty report still carries an (empty) usage array.
        let empty = serde_json::to_value(Report::new()).unwrap();
        assert!(empty["usage"]["entries"].as_array().unwrap().is_empty());
    }

    #[test]
    fn round_trips_a_body_report() {
        let report = Report::new().insert_body::<Text>(vec![text_entity("EMAIL_ADDRESS")]);
        let back = round_trip(&report, &text_registry());

        let body = back.entities::<Text>().expect("body reconstructed as Text");
        assert_eq!(body.len(), 1);
        assert_eq!(body[0].label, LabelRef::new("EMAIL_ADDRESS"));
        assert!(body[0].audit.verify().is_ok(), "the audit trail survives");
    }

    /// The public [`Report::deserializer`] rebuilds a report with no orchestrator
    /// — no analyzers, anonymizers, or codec registry.
    #[test]
    fn report_deserializer_rebuilds_without_an_orchestrator() {
        let report = Report::new().insert_body::<Text>(vec![text_entity("EMAIL_ADDRESS")]);
        let json = serde_json::to_string(&report).unwrap();

        let mut de = serde_json::Deserializer::from_str(&json);
        let back = Report::deserializer()
            .with_modality::<Text>()
            .deserialize(&mut de)
            .expect("rebuilt");
        let body = back.entities::<Text>().expect("body reconstructed as Text");
        assert_eq!(body[0].label, LabelRef::new("EMAIL_ADDRESS"));
    }

    #[test]
    fn round_trips_parts_keyed_by_id() {
        let part = PartId::from("word/media/image1.png".to_owned());
        let report = Report::new()
            .insert_body::<Text>(vec![text_entity("A")])
            .insert_part::<Text>(part.clone(), vec![text_entity("B")]);
        let back = round_trip(&report, &text_registry());

        assert!(back.entities::<Text>().is_some(), "body present");
        let p = back
            .part_entities::<Text>(&part)
            .expect("part reconstructed");
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].label, LabelRef::new("B"));
    }

    #[test]
    fn empty_report_round_trips() {
        let back = round_trip(&Report::new(), &text_registry());
        assert!(back.body.is_none());
        assert!(back.parts.is_empty());
    }

    /// A *non-empty* group naming a modality the registry does not know is
    /// rejected — those entities may carry reviewer edits, and silently dropping
    /// them would lose that work. (The array element need not be a real entity:
    /// the emptiness check fires before any parse.)
    #[test]
    fn unregistered_modality_with_entities_is_rejected() {
        let json = r#"{"body":{"modality":"audio","entities":[{}]},"parts":{}}"#;
        let mut de = serde_json::Deserializer::from_str(json);
        let err = match ReportSeed::new(&text_registry()).deserialize(&mut de) {
            Ok(_) => panic!("a non-empty unregistered group must be rejected"),
            Err(e) => e,
        };
        assert!(
            err.to_string()
                .contains("no registered modality for `audio`"),
            "got: {err}",
        );
    }

    /// An *empty* group naming an unregistered modality is skipped, not an error:
    /// an orchestrator without that pipeline could not have redacted the part
    /// anyway, and skipping an empty group loses nothing — matching how `analyze`
    /// ignores a part whose modality has no pipeline. A body skips to `None`; a
    /// part is dropped from the map.
    #[test]
    fn unregistered_empty_group_is_skipped() {
        // An audio body and an audio part, both empty, against a text-only
        // registry: the report reconstructs with no body and no parts.
        let json = r#"{"body":{"modality":"audio","entities":[]},"parts":{"a/b.wav":{"modality":"audio","entities":[]}}}"#;
        let mut de = serde_json::Deserializer::from_str(json);
        let back = match ReportSeed::new(&text_registry()).deserialize(&mut de) {
            Ok(report) => report,
            Err(e) => panic!("empty unregistered groups must be skipped: {e}"),
        };
        assert!(back.body.is_none(), "empty unregistered body skipped");
        assert!(back.parts.is_empty(), "empty unregistered part skipped");
    }

    /// A review layer's JSON tooling may reorder object keys, so a group with
    /// `entities` before `modality` must still parse — with *real* buffered
    /// entities, not just an empty array — and the entity data must survive.
    #[test]
    fn group_fields_may_arrive_in_any_order() {
        // A real entity array, then `entities` placed before `modality` — the
        // reverse of what we emit.
        let entities = serde_json::to_string(&vec![text_entity("EMAIL_ADDRESS")]).unwrap();
        let json =
            format!(r#"{{"parts":{{}},"body":{{"entities":{entities},"modality":"text"}}}}"#);
        let mut de = serde_json::Deserializer::from_str(&json);
        let report = match ReportSeed::new(&text_registry()).deserialize(&mut de) {
            Ok(report) => report,
            Err(e) => panic!("key order must not matter: {e}"),
        };
        let body = report.entities::<Text>().expect("body reconstructed");
        assert_eq!(body.len(), 1, "the buffered entity survived reordering");
        assert_eq!(body[0].label, LabelRef::new("EMAIL_ADDRESS"));
        assert!(body[0].audit.verify().is_ok(), "audit data survived");
    }

    /// An unknown group field (a later format version) is ignored, matching the
    /// report-level policy — the wire format can grow additively.
    #[test]
    fn unknown_group_fields_are_ignored() {
        let json = r#"{"body":{"modality":"text","entities":[],"future":42},"parts":{}}"#;
        let mut de = serde_json::Deserializer::from_str(json);
        assert!(
            ReportSeed::new(&text_registry())
                .deserialize(&mut de)
                .is_ok(),
            "unknown group fields must be ignored",
        );
    }

    /// A duplicate top-level field is rejected.
    #[test]
    fn duplicate_report_field_is_rejected() {
        let json = r#"{"body":null,"body":null,"parts":{}}"#;
        let mut de = serde_json::Deserializer::from_str(json);
        let err = match ReportSeed::new(&text_registry()).deserialize(&mut de) {
            Ok(_) => panic!("duplicate `body` must be rejected"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("duplicate field"), "got: {err}");
    }

    /// A repeated part id is rejected: silently overwriting the earlier group
    /// would drop its (possibly reviewer-edited) entities, and `anonymize_with`
    /// would apply only the later group. The first entry here is a *skipped*
    /// (unregistered-and-empty `audio`) group, so this also verifies a skipped
    /// `None` still reserves its id and cannot be overwritten.
    #[test]
    fn duplicate_part_id_is_rejected() {
        let json = concat!(
            r#"{"body":null,"parts":{"#,
            r#""a/b.txt":{"modality":"audio","entities":[]},"#,
            r#""a/b.txt":{"modality":"text","entities":[]}}}"#,
        );
        let mut de = serde_json::Deserializer::from_str(json);
        let err = match ReportSeed::new(&text_registry()).deserialize(&mut de) {
            Ok(_) => panic!("a duplicate part id must be rejected"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("duplicate part id `a/b.txt`"),
            "got: {err}",
        );
    }

    /// An image body's OCR [`Layout`] survives the [`ArtifactSet`] serialize
    /// round trip — the whole point of persisting it beside the report is that a
    /// re-run reads the same OCR without re-invoking the model. A stored *empty*
    /// artifact (an image OCR'd to no text) survives too, distinct from an
    /// un-enriched payload that was never inserted.
    #[test]
    fn round_trips_an_artifact_set() {
        use elide_core::modality::image::{Image, ImageLocation, Layout, LayoutBlock};
        use elide_core::primitive::{BoundingBox, Point};

        let bbox = BoundingBox::from_origin_size(Point::new(0.0, 0.0), 100.0, 20.0);
        let layout = Layout::new(vec![LayoutBlock::new(ImageLocation::new(bbox), "hi Alice")]);
        // The body carries a real Layout; a part was enriched to an *empty*
        // Layout (an image with no text) — both are stored and both must survive.
        let set = ArtifactSet::new()
            .insert_body::<Image>(layout.clone())
            .insert_part::<Image>(PartId::from("blank".to_owned()), Layout::default());

        // Both reach the wire: the non-empty body Layout and the enriched-empty
        // part Layout (omitting the latter would force a needless re-OCR).
        let value = serde_json::to_value(&set).unwrap();
        assert_eq!(value["body"]["modality"], "image");
        assert!(value["body"]["artifact"].is_object());
        assert!(
            value["parts"]["blank"]["artifact"].is_object(),
            "an enriched-empty artifact must survive, got {value:#}",
        );

        // Both reconstruct through the registry: the body as the same Layout,
        // and the part as an *empty* Layout that is present (Some) — so a re-run
        // seeds it and the enricher skips, rather than re-OCR'ing a blank image.
        let mut registry = ModalityRegistry::default();
        registry.register::<Image>();
        let json = serde_json::to_string(&set).unwrap();
        let mut de = serde_json::Deserializer::from_str(&json);
        let back = registry.deserialize_artifacts(&mut de).expect("artifacts");
        let restored = back.body::<Image>().expect("layout reconstructed");
        assert_eq!(restored, &layout);
        assert_eq!(restored.text(), "hi Alice");
        let part = back
            .part::<Image>(&PartId::from("blank".to_owned()))
            .expect("the enriched-empty part is present, not dropped");
        assert!(part.is_empty(), "it round-trips as the empty Layout it was");
    }

    /// A group whose `artifact` field is `null` — or absent entirely — is
    /// dropped, not rejected. An artifact carries no reviewer edits, so a payload
    /// that reconstructs to nothing is treated as "not enriched" (a re-run
    /// re-enriches), matching the unregistered-modality drop. Our own serializer
    /// never emits this; hand-authored input can.
    #[test]
    fn a_null_or_absent_artifact_is_dropped() {
        use elide_core::modality::image::Image;

        let mut registry = ModalityRegistry::default();
        registry.register::<Image>();

        for json in [
            r#"{ "body": { "modality": "image", "artifact": null }, "parts": {} }"#,
            r#"{ "body": { "modality": "image" }, "parts": {} }"#,
            r#"{ "body": null, "parts": { "p": { "modality": "image", "artifact": null } } }"#,
        ] {
            let mut de = serde_json::Deserializer::from_str(json);
            let set = registry
                .deserialize_artifacts(&mut de)
                .expect("a null/absent artifact deserializes without error");
            assert!(
                set.body::<Image>().is_none(),
                "the body artifact was dropped: {json}",
            );
            assert!(
                set.part::<Image>(&PartId::from("p".to_owned())).is_none(),
                "the part artifact was dropped: {json}",
            );
        }
    }
}
