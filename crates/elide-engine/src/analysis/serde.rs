//! The serde wire form for the analysis views: the [`Report`]'s
//! `serde::Serialize` and the seed visitors that reconstruct both a [`Report`]
//! and an [`ArtifactSet`] from their `{ parts: [..] }` wire shape.
//!
//! The serialized report is the part-list `{ parts: [ { id, modality, X } ] }`
//! view a review layer consumes: every part, a named document's own content
//! and every container part within it, is one array entry, keyed by its
//! [`PartId`] path (a segment array, never string-joined, so a nested part can
//! never collide with a same-named part in another document). Each entry is
//! tagged with its [`modality`](elide_core::modality::Modality::NAME) so the
//! orchestrator can route it back on deserialization. Reconstruction is driven
//! by the [`ModalityRegistry`](super::registry::ModalityRegistry): the wire form
//! drops the concrete modality type, deserialization is not object-safe, so each
//! group is buffered and replayed through the per-modality parser resolved from
//! the registry.
//!
//! Both views share the same wire shape, `{ parts: [..] }` of `{ id, modality,
//! X }` entries, so one generic seed family drives both, parameterized by a
//! [`Leaf`]: the entities of a [`Report`] or the artifact of an [`ArtifactSet`].
//! The two differ only at the leaf: the field name (`entities` / `artifact`),
//! which registry parser to run, and what an *unregistered* modality means. For
//! entities, an unregistered group is skipped when empty (nothing to lose,
//! matching how `analyze` ignores an unmatched part) but a hard error when
//! non-empty, its entities may carry reviewer edits that silently dropping the
//! group would lose, and an absent/null `entities` field is likewise rejected.
//! An artifact carries no reviewer-editable state, so anything that reconstructs
//! to nothing, an unregistered modality, or an absent/null `artifact` field,
//! is simply dropped, and a re-run re-enriches.
//!
//! [`Report`]: super::report::Report
//! [`ArtifactSet`]: super::artifacts::ArtifactSet

use std::any::TypeId;
use std::marker::PhantomData;

use serde::de::{
    DeserializeSeed, Deserializer, Error as DeError, IntoDeserializer, MapAccess, Visitor,
};
use serde_value::Value;

use super::artifacts::ArtifactSet;
use super::group::{ArtifactGroup, EntityGroup};
use super::registry::{ModalityEntry, ModalityRegistry};
use super::report::{PartReport, Report};
use crate::PartId;

/// A part's [`PartId`] path serialized as a segment array, the wire key. A
/// path is never string-joined (a segment can contain any delimiter), so it
/// rides as a list of strings.
struct PathField<'a>(&'a PartId);
impl serde::Serialize for PathField<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.0.segments())
    }
}

impl serde::Serialize for Report {
    /// Serialize to `{ parts: [ { id: [seg..], modality, entities } ] }` (plus
    /// `usage` under that feature). Every part, a document's own content and
    /// every nested container part, is one entry, keyed by its full path so no
    /// two collide. Each carries its modality name so it can be parsed back into
    /// the right `Vec<Entity<M>>`.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        // One part entry: `{ id: [seg..], modality, entities }`. The entities
        // serialize through erasure; `modality` tags which `M` to parse back as.
        struct Entry<'a>(&'a PartId, &'a dyn EntityGroup);
        impl serde::Serialize for Entry<'_> {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                struct Entities<'a>(&'a dyn EntityGroup);
                impl serde::Serialize for Entities<'_> {
                    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                        erased_serde::serialize(self.0, s)
                    }
                }
                let mut state = s.serialize_struct("PartEntry", 3)?;
                state.serialize_field("id", &PathField(self.0))?;
                state.serialize_field("modality", self.1.modality_name())?;
                state.serialize_field("entities", &Entities(self.1))?;
                state.end()
            }
        }

        let mut parts: Vec<Entry<'_>> = self
            .parts
            .iter()
            .map(|(id, p)| Entry(id, p.entities.as_ref()))
            .collect();
        // Deterministic wire output: `parts` is a `HashMap` (random iteration
        // order), and the array position is observable. Sort by path so identical
        // reports serialize identically and the order follows the `PartId` tree.
        parts.sort_unstable_by(|a, b| a.0.segments().cmp(b.0.segments()));

        // `usage` is the second field only under the `usage` feature.
        #[cfg(feature = "usage")]
        let field_count = 2;
        #[cfg(not(feature = "usage"))]
        let field_count = 1;

        let mut state = serializer.serialize_struct("Report", field_count)?;
        state.serialize_field("parts", &parts)?;
        #[cfg(feature = "usage")]
        state.serialize_field("usage", &self.usage)?;
        state.end()
    }
}

// ---- The generic deserialization core -------------------------------------
//
// Both a `Report` (entities) and an `ArtifactSet` (artifacts) deserialize from
// the same `{ parts: [..] }` wire shape of `{ id, modality, X }` entries. A
// `Leaf` captures everything that differs between the two: the field name, the
// struct names, which registry parser reconstructs a buffered group, and how a
// parsed group is stored into the final set. The seeds below are generic over it.

/// What the two views (a [`Report`]'s entities, an [`ArtifactSet`]'s artifacts)
/// differ by. Everything else, the buffered, order-independent `{ id, modality,
/// X }` traversal and the `{ parts: [..] }` assembly, is shared.
pub(super) trait Leaf {
    /// One reconstructed group of this leaf: the concrete boxed value plus the
    /// routing metadata the set entry keys on.
    type Parsed;
    /// The set this leaf reconstructs: [`Report`] or [`ArtifactSet`].
    type Set;

    /// The wire field carrying this leaf's payload inside a part entry:
    /// `"entities"` for a report, `"artifact"` for an artifact set.
    const FIELD: &'static str;
    /// The part-entry struct name: `"PartEntry"` / `"ArtifactEntry"`.
    const GROUP_NAME: &'static str;
    /// The whole-set struct name: `"Report"` / `"ArtifactSet"`.
    const SET_NAME: &'static str;
    /// What `expecting` writes for one part entry.
    const GROUP_EXPECTING: &'static str;
    /// What `expecting` writes for the whole set.
    const SET_EXPECTING: &'static str;

    /// Reconstruct one entry's payload, applying this leaf's unregistered-modality
    /// policy. `name` is the entry's modality tag and `value` its buffered
    /// payload (already read from the `FIELD` field). Returns `None` when the
    /// entry is skipped.
    fn parse<E: DeError>(
        entry: Option<ModalityEntry>,
        name: &str,
        value: Value,
    ) -> Result<Option<Self::Parsed>, E>;

    /// An empty set to fill in.
    fn empty() -> Self::Set;
    /// Store a reconstructed part group, keyed by `id`, into the set.
    fn set_part(set: &mut Self::Set, id: PartId, parsed: Self::Parsed);
}

/// The report leaf: a group's `entities` reconstruct as a boxed
/// `Vec<Entity<M>>`. An unregistered modality is skipped only when empty; a
/// non-empty one is a hard error, since its entities may carry reviewer edits.
pub(super) struct EntityLeaf;

/// One reconstructed entity group: the entities and the routing [`TypeId`] the
/// report entry keys on (matching [`PartReport::modality`]).
pub(super) struct ParsedGroup {
    modality: TypeId,
    entities: Box<dyn EntityGroup>,
}

impl Leaf for EntityLeaf {
    type Parsed = ParsedGroup;
    type Set = Report;

    const FIELD: &'static str = "entities";
    const GROUP_EXPECTING: &'static str = "a { id, modality, entities } part entry";
    const GROUP_NAME: &'static str = "PartEntry";
    const SET_EXPECTING: &'static str = "a { parts: [..] } report";
    const SET_NAME: &'static str = "Report";

    fn parse<E: DeError>(
        entry: Option<ModalityEntry>,
        name: &str,
        value: Value,
    ) -> Result<Option<ParsedGroup>, E> {
        // Entities are required, an absent/null `entities` field (`Value::Unit`)
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

/// Whether a buffered `entities` value is an empty list, the group carries no
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
/// `M::Artifact`. An unregistered *or* absent modality simply yields nothing,
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
    const GROUP_EXPECTING: &'static str = "a { id, modality, artifact } part entry";
    const GROUP_NAME: &'static str = "ArtifactEntry";
    const SET_EXPECTING: &'static str = "a { parts: [..] } artifact set";
    const SET_NAME: &'static str = "ArtifactSet";

    fn parse<E: DeError>(
        entry: Option<ModalityEntry>,
        _name: &str,
        value: Value,
    ) -> Result<Option<ParsedArtifact>, E> {
        // A dropped artifact loses no work (a re-run re-enriches), so anything
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

    fn set_part(set: &mut ArtifactSet, id: PartId, parsed: ParsedArtifact) {
        set.set_part(id, parsed.modality, parsed.modality_name, parsed.artifact);
    }
}

/// A serialized part entry: `{ id: [seg..], modality, X }` (`X` is `entities`
/// or `artifact`). Deserializes by buffering all three fields (in any order, a
/// review layer may reorder keys), resolving the `modality`'s registered entry,
/// then running that entry's parser over the buffered payload. Yields the entry's
/// full [`PartId`] and its parsed group (`None` when the leaf's
/// unregistered-modality policy skips the group, the id is still returned so a
/// later duplicate is caught).
struct PartEntrySeed<'a, L> {
    registry: &'a ModalityRegistry,
    _leaf: PhantomData<L>,
}

impl<'a, L> PartEntrySeed<'a, L> {
    fn new(registry: &'a ModalityRegistry) -> Self {
        Self {
            registry,
            _leaf: PhantomData,
        }
    }
}

impl<'de, L: Leaf> DeserializeSeed<'de> for PartEntrySeed<'_, L> {
    type Value = (PartId, Option<L::Parsed>);

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_struct(L::GROUP_NAME, &["id", "modality", L::FIELD], self)
    }
}

impl<'de, L: Leaf> Visitor<'de> for PartEntrySeed<'_, L> {
    type Value = (PartId, Option<L::Parsed>);

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(L::GROUP_EXPECTING)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        // Order-independent: a review layer's JSON tooling may reorder keys, so
        // the payload can arrive before `modality`. Buffer all three, then resolve
        // the parser and parse the payload after the map is fully read. Unknown
        // fields are ignored (as at the set level) so the format can grow.
        let mut id: Option<Vec<String>> = None;
        let mut modality: Option<String> = None;
        let mut payload: Option<Value> = None;
        while let Some(key) = map.next_key::<String>()? {
            if key == "id" {
                if id.is_some() {
                    return Err(DeError::duplicate_field("id"));
                }
                id = Some(map.next_value()?);
            } else if key == "modality" {
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
        let segments = id.ok_or_else(|| DeError::missing_field("id"))?;
        if segments.is_empty() {
            return Err(DeError::custom("part id must have at least one segment"));
        }
        let part_id = PartId::from_segments(segments);
        let name = modality.ok_or_else(|| DeError::missing_field("modality"))?;
        // An absent payload field is `Value::Unit` (the same value a JSON `null`
        // yields), so `Leaf::parse` decides what a missing payload means for its
        // side, entities require one, an artifact treats it as "not enriched".
        let payload = payload.unwrap_or(Value::Unit);
        let parsed = L::parse(self.registry.entry(&name), &name, payload)?;
        Ok((part_id, parsed))
    }
}

/// `parts` is an array of `{ id, modality, X }` entries, each keyed by its full
/// [`PartId`] path.
struct PartsSeed<'a, L> {
    registry: &'a ModalityRegistry,
    _leaf: PhantomData<L>,
}

impl<'de, L: Leaf> DeserializeSeed<'de> for PartsSeed<'_, L> {
    type Value = L::Set;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de, L: Leaf> Visitor<'de> for PartsSeed<'_, L> {
    type Value = L::Set;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an array of part entries")
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut set = L::empty();
        // Full paths already seen, a repeated path would silently overwrite the
        // earlier group (and its entity edits), so reject it. A skipped
        // (unregistered-and-empty) entry still records its path so a later
        // duplicate is caught.
        let mut seen: std::collections::HashSet<PartId> = std::collections::HashSet::new();
        while let Some((id, parsed)) =
            seq.next_element_seed(PartEntrySeed::<L>::new(self.registry))?
        {
            if !seen.insert(id.clone()) {
                return Err(DeError::custom(format!("duplicate part id `{id}`")));
            }
            if let Some(parsed) = parsed {
                L::set_part(&mut set, id, parsed);
            }
        }
        Ok(set)
    }
}

/// The whole-set seed: `{ parts: [..] }`, each entry parsed through the registry.
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

/// Reconstructs a [`Report`] from its `{ parts: [..] }` wire form.
pub(super) type ReportSeed<'a> = SetSeed<'a, EntityLeaf>;
/// Reconstructs an [`ArtifactSet`] from its `{ parts: [..] }` wire form.
pub(super) type ArtifactSetSeed<'a> = SetSeed<'a, ArtifactLeaf>;

impl<'de, L: Leaf> serde::de::DeserializeSeed<'de> for SetSeed<'_, L> {
    type Value = L::Set;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<L::Set, D::Error> {
        deserializer.deserialize_struct(L::SET_NAME, &["parts"], self)
    }
}

impl<'de, L: Leaf> Visitor<'de> for SetSeed<'_, L> {
    type Value = L::Set;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(L::SET_EXPECTING)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<L::Set, A::Error> {
        let mut set: Option<L::Set> = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "parts" => {
                    if set.is_some() {
                        return Err(DeError::duplicate_field("parts"));
                    }
                    set = Some(map.next_value_seed(PartsSeed::<L> {
                        registry: self.registry,
                        _leaf: PhantomData,
                    })?);
                }
                // `usage` (and any future field) is ignored: it is derived
                // analysis output, not editable review state. `parts` is optional
                //, a set with no parts is valid.
                _ => {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
        }
        Ok(set.unwrap_or_else(L::empty))
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::LabelRef;
    use elide_core::modality::text::Text;
    use serde::de::DeserializeSeed;

    use super::super::test_support::{doc, text_entity};
    use super::*;
    use crate::PartId;

    /// A registry with `Text` registered, the unit under test needs only the
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
    fn serializes_to_part_list_view() {
        // The `{ parts: [..] }` shape is exercised end to end (with a real
        // container) in the docx integration test; here we check the entry shape
        // and the empty-parts case directly.
        let report = Report::new().insert_part::<Text>(doc(), vec![text_entity("EMAIL_ADDRESS")]);

        let value = serde_json::to_value(&report).unwrap();
        // parts is an array of `{ id, modality, entities }` entries.
        let parts = value["parts"].as_array().expect("parts is an array");
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0]["id"], serde_json::json!(["document"]));
        assert_eq!(parts[0]["modality"], "text");
        assert_eq!(parts[0]["entities"][0]["label"], "EMAIL_ADDRESS");

        // No parts → an empty array.
        let empty = serde_json::to_value(Report::new()).unwrap();
        assert!(empty["parts"].as_array().unwrap().is_empty());
    }

    /// A nested part serializes its full path as a segment array, so a part
    /// nested inside one document never collides with a same-named part in
    /// another, the collision the old `last_segment` wire form could hit.
    #[test]
    fn serializes_a_nested_path_as_a_segment_array() {
        let nested = PartId::new("scan-A.docx").child("word/media/image1.png");
        let report = Report::new().insert_part::<Text>(nested, vec![text_entity("EMAIL_ADDRESS")]);

        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(
            value["parts"][0]["id"],
            serde_json::json!(["scan-A.docx", "word/media/image1.png"]),
        );
    }

    /// The hand-written [`JsonSchema`](super::super::schema) must accept what
    /// [`Serialize`] produces, the drift guard. A populated report (a document
    /// plus a nested container part), a single empty document, and an empty
    /// report all validate against `schema_for!(Report)`; any divergence between
    /// the two hand-written impls fails here rather than silently shipping a
    /// schema that lies.
    #[cfg(feature = "schema")]
    #[test]
    fn serialized_reports_validate_against_the_schema() {
        let schema = serde_json::to_value(schemars::schema_for!(Report)).unwrap();

        let with_part = Report::new()
            .insert_part::<Text>(doc(), vec![text_entity("EMAIL_ADDRESS")])
            .insert_part::<Text>(
                doc().child("word/media/image1.png"),
                vec![text_entity("PHONE_NUMBER")],
            );

        for report in [
            with_part,
            Report::new().insert_part::<Text>(doc(), Vec::new()),
            Report::new(),
        ] {
            let json = serde_json::to_value(&report).unwrap();
            if let Err(e) = jsonschema::validate(&schema, &json) {
                panic!("serialized report does not match its schema: {e}\n{json:#}");
            }
        }
    }

    /// The artifact-side drift guard, mirroring the report's: a serialized
    /// [`ArtifactSet`] validates against its hand-written schema, so a change to
    /// one that is not made to the other fails here rather than handing a
    /// generated client a schema that lies about the wire.
    ///
    /// Uses an `Image` artifact for a richer payload than text's `NoArtifact`,
    /// so it needs `image` as well as `schema`.
    #[cfg(all(feature = "schema", feature = "image"))]
    #[test]
    fn serialized_artifact_sets_validate_against_the_schema() {
        use elide_core::modality::image::{Image, ImageLocation, Layout, LayoutBlock};
        use elide_core::primitive::{BoundingBox, Point};

        let schema = serde_json::to_value(schemars::schema_for!(ArtifactSet)).unwrap();

        let bbox = BoundingBox::from_origin_size(Point::new(0.0, 0.0), 100.0, 20.0);
        let layout = Layout::new(vec![LayoutBlock::new(ImageLocation::new(bbox), "hi Alice")]);
        let with_part = ArtifactSet::new()
            .insert_body::<Image>(doc(), layout)
            .insert_part::<Image>(doc().child("blank"), Layout::default());

        for set in [
            with_part,
            ArtifactSet::new().insert_body::<Image>(doc(), Layout::default()),
            ArtifactSet::new(),
        ] {
            let json = serde_json::to_value(&set).unwrap();
            if let Err(e) = jsonschema::validate(&schema, &json) {
                panic!("serialized artifact set does not match its schema: {e}\n{json:#}");
            }
        }
    }

    #[cfg(feature = "usage")]
    #[test]
    fn serializes_usage_entries() {
        use std::time::Duration;

        use elide_core::primitive::Usage;
        use elide_core::recognition::RecognizerId;

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
    fn round_trips_a_sole_document_report() {
        let report = Report::new().insert_part::<Text>(doc(), vec![text_entity("EMAIL_ADDRESS")]);
        let back = round_trip(&report, &text_registry());

        let entities = back
            .entities::<Text>()
            .expect("sole document reconstructed");
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].label, LabelRef::new("EMAIL_ADDRESS"));
        assert!(
            entities[0].audit.verify().is_ok(),
            "the audit trail survives"
        );
    }

    /// The public [`Report::deserializer`] rebuilds a report with no orchestrator,
    /// no analyzers, anonymizers, or codec registry.
    #[test]
    fn report_deserializer_rebuilds_without_an_orchestrator() {
        let report = Report::new().insert_part::<Text>(doc(), vec![text_entity("EMAIL_ADDRESS")]);
        let json = serde_json::to_string(&report).unwrap();

        let mut de = serde_json::Deserializer::from_str(&json);
        let back = Report::deserializer()
            .with_modality::<Text>()
            .deserialize(&mut de)
            .expect("rebuilt");
        let entities = back
            .entities::<Text>()
            .expect("sole document reconstructed");
        assert_eq!(entities[0].label, LabelRef::new("EMAIL_ADDRESS"));
    }

    #[test]
    fn round_trips_a_nested_part_by_full_path() {
        // A document with a nested container part, its full path must survive.
        let nested = doc().child("word/media/image1.png");
        let report = Report::new()
            .insert_part::<Text>(doc(), vec![text_entity("A")])
            .insert_part::<Text>(nested.clone(), vec![text_entity("B")]);
        let back = round_trip(&report, &text_registry());

        assert!(back.entities::<Text>().is_some(), "sole document present");
        let p = back
            .part_entities::<Text>(&nested)
            .expect("nested part reconstructed by its full path");
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].label, LabelRef::new("B"));
    }

    /// Two documents that share a nested part's local id keep distinct full
    /// paths across the round trip, the collision the segment-array wire form
    /// prevents.
    #[test]
    fn round_trips_same_named_nested_parts_in_two_documents() {
        let a = PartId::new("scan-A.docx").child("word/media/image1.png");
        let b = PartId::new("scan-B.docx").child("word/media/image1.png");
        let report = Report::new()
            .insert_part::<Text>(a.clone(), vec![text_entity("A")])
            .insert_part::<Text>(b.clone(), vec![text_entity("B")]);
        let back = round_trip(&report, &text_registry());

        assert_eq!(
            back.part_entities::<Text>(&a).map(|e| e[0].label.as_str()),
            Some("A"),
        );
        assert_eq!(
            back.part_entities::<Text>(&b).map(|e| e[0].label.as_str()),
            Some("B"),
        );
    }

    #[test]
    fn empty_report_round_trips() {
        let back = round_trip(&Report::new(), &text_registry());
        assert!(back.parts.is_empty());
    }

    /// A *non-empty* entry naming a modality the registry does not know is
    /// rejected, those entities may carry reviewer edits, and silently dropping
    /// them would lose that work. (The array element need not be a real entity:
    /// the emptiness check fires before any parse.)
    #[test]
    fn unregistered_modality_with_entities_is_rejected() {
        let json = r#"{"parts":[{"id":["a"],"modality":"audio","entities":[{}]}]}"#;
        let mut de = serde_json::Deserializer::from_str(json);
        let err = match ReportSeed::new(&text_registry()).deserialize(&mut de) {
            Ok(_) => panic!("a non-empty unregistered entry must be rejected"),
            Err(e) => e,
        };
        assert!(
            err.to_string()
                .contains("no registered modality for `audio`"),
            "got: {err}",
        );
    }

    /// An *empty* entry naming an unregistered modality is skipped, not an error:
    /// an orchestrator without that pipeline could not have redacted the part
    /// anyway, and skipping an empty group loses nothing, matching how `analyze`
    /// ignores a part whose modality has no pipeline. The part is dropped.
    #[test]
    fn unregistered_empty_group_is_skipped() {
        // Two empty audio parts against a text-only registry: the report
        // reconstructs with no parts.
        let json = r#"{"parts":[{"id":["a"],"modality":"audio","entities":[]},{"id":["a","b.wav"],"modality":"audio","entities":[]}]}"#;
        let mut de = serde_json::Deserializer::from_str(json);
        let back = match ReportSeed::new(&text_registry()).deserialize(&mut de) {
            Ok(report) => report,
            Err(e) => panic!("empty unregistered groups must be skipped: {e}"),
        };
        assert!(back.parts.is_empty(), "empty unregistered parts skipped");
    }

    /// A review layer's JSON tooling may reorder object keys, so an entry with
    /// `entities` before `modality` (and `id` last) must still parse, with
    /// *real* buffered entities, not just an empty array, and the data survive.
    #[test]
    fn entry_fields_may_arrive_in_any_order() {
        // A real entity array, with `entities` before `modality` and `id` last ,
        // the reverse of what we emit.
        let entities = serde_json::to_string(&vec![text_entity("EMAIL_ADDRESS")]).unwrap();
        let json = format!(
            r#"{{"parts":[{{"entities":{entities},"modality":"text","id":["document"]}}]}}"#
        );
        let mut de = serde_json::Deserializer::from_str(&json);
        let report = match ReportSeed::new(&text_registry()).deserialize(&mut de) {
            Ok(report) => report,
            Err(e) => panic!("key order must not matter: {e}"),
        };
        let entities = report.entities::<Text>().expect("entry reconstructed");
        assert_eq!(entities.len(), 1, "the buffered entity survived reordering");
        assert_eq!(entities[0].label, LabelRef::new("EMAIL_ADDRESS"));
        assert!(entities[0].audit.verify().is_ok(), "audit data survived");
    }

    /// An unknown entry field (a later format version) is ignored, matching the
    /// set-level policy, the wire format can grow additively.
    #[test]
    fn unknown_entry_fields_are_ignored() {
        let json = r#"{"parts":[{"id":["document"],"modality":"text","entities":[],"future":42}]}"#;
        let mut de = serde_json::Deserializer::from_str(json);
        assert!(
            ReportSeed::new(&text_registry())
                .deserialize(&mut de)
                .is_ok(),
            "unknown entry fields must be ignored",
        );
    }

    /// A duplicate top-level field is rejected.
    #[test]
    fn duplicate_report_field_is_rejected() {
        let json = r#"{"parts":[],"parts":[]}"#;
        let mut de = serde_json::Deserializer::from_str(json);
        let err = match ReportSeed::new(&text_registry()).deserialize(&mut de) {
            Ok(_) => panic!("duplicate `parts` must be rejected"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("duplicate field"), "got: {err}");
    }

    /// A repeated part path is rejected: silently overwriting the earlier group
    /// would drop its (possibly reviewer-edited) entities, and `anonymize_with`
    /// would apply only the later group. The first entry here is a *skipped*
    /// (unregistered-and-empty `audio`) group, so this also verifies a skipped
    /// entry still reserves its path and cannot be overwritten.
    #[test]
    fn duplicate_part_id_is_rejected() {
        let json = concat!(
            r#"{"parts":["#,
            r#"{"id":["a","b.txt"],"modality":"audio","entities":[]},"#,
            r#"{"id":["a","b.txt"],"modality":"text","entities":[]}]}"#,
        );
        let mut de = serde_json::Deserializer::from_str(json);
        let err = match ReportSeed::new(&text_registry()).deserialize(&mut de) {
            Ok(_) => panic!("a duplicate part id must be rejected"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("duplicate part id `a › b.txt`"),
            "got: {err}",
        );
    }

    /// A part entry with an empty `id` array is rejected, a part must have at
    /// least one path segment (there is no "top-level document" without a name).
    #[test]
    fn empty_part_id_is_rejected() {
        let json = r#"{"parts":[{"id":[],"modality":"text","entities":[]}]}"#;
        let mut de = serde_json::Deserializer::from_str(json);
        let err = match ReportSeed::new(&text_registry()).deserialize(&mut de) {
            Ok(_) => panic!("an empty part id must be rejected"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("at least one segment"),
            "got: {err}",
        );
    }

    /// An image document's OCR [`Layout`] survives the [`ArtifactSet`] serialize
    /// round trip, the whole point of persisting it beside the report is that a
    /// re-run reads the same OCR without re-invoking the model. A stored *empty*
    /// artifact (an image OCR'd to no text) survives too, distinct from an
    /// un-enriched payload that was never inserted.
    #[cfg(feature = "image")]
    #[test]
    fn round_trips_an_artifact_set() {
        use elide_core::modality::image::{Image, ImageLocation, Layout, LayoutBlock};
        use elide_core::primitive::{BoundingBox, Point};

        let bbox = BoundingBox::from_origin_size(Point::new(0.0, 0.0), 100.0, 20.0);
        let layout = Layout::new(vec![LayoutBlock::new(ImageLocation::new(bbox), "hi Alice")]);
        // The sole document carries a real Layout; a nested part was enriched to
        // an *empty* Layout (an image with no text), both stored, both survive.
        let blank = doc().child("blank");
        let set = ArtifactSet::new()
            .insert_body::<Image>(doc(), layout.clone())
            .insert_part::<Image>(blank.clone(), Layout::default());

        // Both reach the wire: the non-empty document Layout and the
        // enriched-empty nested Layout (omitting the latter forces a needless re-OCR).
        let value = serde_json::to_value(&set).unwrap();
        let parts = value["parts"].as_array().expect("parts is an array");
        assert_eq!(parts.len(), 2);
        assert!(
            parts.iter().all(|p| p["artifact"].is_object()),
            "every artifact reaches the wire, got {value:#}",
        );
        assert!(
            parts
                .iter()
                .any(|p| p["id"] == serde_json::json!(["document"])),
            "the sole document's artifact is keyed by its name",
        );

        // Both reconstruct through the registry: the document as the same Layout,
        // and the nested part as an *empty* Layout that is present (Some), so a
        // re-run seeds it and the enricher skips, rather than re-OCR'ing a blank.
        let mut registry = ModalityRegistry::default();
        registry.register::<Image>();
        let json = serde_json::to_string(&set).unwrap();
        let mut de = serde_json::Deserializer::from_str(&json);
        let back = registry.deserialize_artifacts(&mut de).expect("artifacts");
        let restored = back.body::<Image>().expect("layout reconstructed");
        assert_eq!(restored, &layout);
        assert_eq!(restored.text(), "hi Alice");
        let part = back
            .part::<Image>(&blank)
            .expect("the enriched-empty part is present, not dropped");
        assert!(part.is_empty(), "it round-trips as the empty Layout it was");
    }

    /// An entry whose `artifact` field is `null`, or absent entirely, is
    /// dropped, not rejected. An artifact carries no reviewer edits, so a payload
    /// that reconstructs to nothing is treated as "not enriched" (a re-run
    /// re-enriches), matching the unregistered-modality drop. Our own serializer
    /// never emits this; hand-authored input can.
    #[cfg(feature = "image")]
    #[test]
    fn a_null_or_absent_artifact_is_dropped() {
        use elide_core::modality::image::Image;

        let mut registry = ModalityRegistry::default();
        registry.register::<Image>();

        for json in [
            r#"{ "parts": [{ "id": ["document"], "modality": "image", "artifact": null }] }"#,
            r#"{ "parts": [{ "id": ["document"], "modality": "image" }] }"#,
            r#"{ "parts": [{ "id": ["p"], "modality": "image", "artifact": null }] }"#,
        ] {
            let mut de = serde_json::Deserializer::from_str(json);
            let set = registry
                .deserialize_artifacts(&mut de)
                .expect("a null/absent artifact deserializes without error");
            assert!(
                set.body::<Image>().is_none(),
                "the document artifact was dropped: {json}",
            );
            assert!(
                set.part::<Image>(&PartId::from("p".to_owned())).is_none(),
                "the part artifact was dropped: {json}",
            );
        }
    }
}
