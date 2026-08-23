//! Reconstructing a [`Report`] from its serialized wire form, driven by the
//! orchestrator's registered modalities.
//!
//! The wire form drops the concrete modality type — a group is just tagged with
//! its [`Modality::NAME`]. Deserialization is not object-safe, so the report
//! cannot recover the type on its own. Instead the [`Orchestrator`] holds a
//! [`GroupRegistry`]: [`with_modality`] registers, per modality name, a parser
//! that deserializes that group as the concrete `Vec<Entity<M>>`. A group naming
//! an unregistered modality is skipped when empty (nothing to lose, matching how
//! `analyze` ignores an unmatched part) but a hard error when non-empty — its
//! entities may carry reviewer edits that silently dropping the group would lose.
//!
//! [`Report`]: super::Report
//! [`Orchestrator`]: crate::Orchestrator
//! [`with_modality`]: crate::Orchestrator::with_modality
//! [`Modality::NAME`]: elide_core::modality::Modality::NAME

use std::any::TypeId;
use std::collections::HashMap;

use elide_core::entity::Entity;
use elide_core::modality::Modality;
use serde::de::{
    DeserializeSeed, Deserializer, Error as DeError, IntoDeserializer, MapAccess, Visitor,
};
use serde_value::Value;

use super::group::EntityGroup;
use super::{BodyReport, PartReport, Report};
use crate::PartId;

/// Parses one erased entity group as its concrete `Vec<Entity<M>>`. Registered
/// per modality name by [`Orchestrator::with_modality`].
///
/// [`Orchestrator::with_modality`]: crate::Orchestrator::with_modality
type GroupParser = fn(
    &mut dyn erased_serde::Deserializer<'_>,
) -> Result<Box<dyn EntityGroup>, erased_serde::Error>;

/// What a registered modality contributes to reconstruction: how to parse its
/// group, and the routing [`TypeId`] the report entry stores for the apply path.
#[derive(Clone, Copy)]
struct ModalityEntry {
    parse: GroupParser,
    type_id: TypeId,
}

/// The orchestrator's per-modality reconstruction registry, keyed by
/// [`Modality::NAME`]. Populated by [`with_modality`] so a deserialized report
/// is reconstructed against exactly the modalities the orchestrator handles.
///
/// [`Modality::NAME`]: elide_core::modality::Modality::NAME
/// [`with_modality`]: crate::Orchestrator::with_modality
#[derive(Default)]
pub(crate) struct GroupRegistry {
    entries: HashMap<&'static str, ModalityEntry>,
}

impl GroupRegistry {
    /// Register modality `M`, keyed by its name. Called by
    /// [`with_modality`](crate::Orchestrator::with_modality) alongside the
    /// pipeline registration.
    pub(crate) fn register<M>(&mut self)
    where
        M: Modality,
        Vec<Entity<M>>: EntityGroup + serde::de::DeserializeOwned,
    {
        self.entries.insert(
            M::NAME,
            ModalityEntry {
                parse: |de| {
                    let entities: Vec<Entity<M>> = erased_serde::deserialize(de)?;
                    Ok(Box::new(entities) as Box<dyn EntityGroup>)
                },
                type_id: TypeId::of::<M>(),
            },
        );
    }

    /// The entry registered for `modality`, or `None` if this orchestrator has
    /// no pipeline for it.
    fn entry(&self, modality: &str) -> Option<ModalityEntry> {
        self.entries.get(modality).copied()
    }
}

/// A parsed group with the routing [`TypeId`] its modality registered.
type ParsedGroup = (TypeId, Box<dyn EntityGroup>);

/// A serialized group envelope: `{ modality, entities }`. Deserializes by
/// buffering both fields (in any order — a review layer may reorder keys),
/// resolving the `modality`'s registered entry, then running that entry's parser
/// over the buffered `entities`.
///
/// Yields `None` — the group is skipped — only when the modality is
/// *unregistered and its `entities` are empty*: an orchestrator without that
/// pipeline could not have redacted the part anyway, and skipping an empty group
/// loses nothing, matching how [`analyze`](crate::Orchestrator::analyze) ignores
/// a part whose modality has no pipeline. A *non-empty* unregistered group is a
/// hard error: it may carry entities a reviewer edited, and silently dropping
/// those would lose their work.
struct GroupSeed<'a> {
    registry: &'a GroupRegistry,
}

impl<'de> DeserializeSeed<'de> for GroupSeed<'_> {
    type Value = Option<ParsedGroup>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_struct("Group", &["modality", "entities"], self)
    }
}

impl<'de> Visitor<'de> for GroupSeed<'_> {
    type Value = Option<ParsedGroup>;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a { modality, entities } group")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        // Order-independent: a review layer's JSON tooling may reorder keys, so
        // `entities` can arrive before `modality`. Buffer both, then resolve the
        // parser and parse the entities after the map is fully read. Unknown
        // fields are ignored (as at the report level) so the format can grow.
        let mut modality: Option<String> = None;
        let mut entities: Option<Value> = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "modality" => {
                    if modality.is_some() {
                        return Err(DeError::duplicate_field("modality"));
                    }
                    modality = Some(map.next_value()?);
                }
                "entities" => {
                    if entities.is_some() {
                        return Err(DeError::duplicate_field("entities"));
                    }
                    // Buffered, not parsed yet: the modality (hence the parser)
                    // may not have been seen. A `Value` round-trips into a
                    // deserializer below.
                    entities = Some(map.next_value()?);
                }
                _ => {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
        }
        let name = modality.ok_or_else(|| DeError::missing_field("modality"))?;
        let entities = entities.ok_or_else(|| DeError::missing_field("entities"))?;

        let Some(entry) = self.registry.entry(&name) else {
            // Unregistered modality: skip only an empty group (nothing to lose,
            // as in `analyze`); reject a non-empty one, whose entities a reviewer
            // may have edited.
            if is_empty_entities(&entities) {
                return Ok(None);
            }
            return Err(DeError::custom(format!(
                "no registered modality for `{name}` (its {} entities would be dropped)",
                entity_count(&entities),
            )));
        };

        // Replay the buffered entities through the modality's parser.
        let mut erased = <dyn erased_serde::Deserializer<'_>>::erase(entities.into_deserializer());
        let group = (entry.parse)(&mut erased).map_err(DeError::custom)?;
        Ok(Some((entry.type_id, group)))
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

/// The whole-report seed: `{ body, parts }`, each group parsed through the
/// registry. Drives [`Orchestrator::deserialize_report`].
///
/// [`Orchestrator::deserialize_report`]: crate::Orchestrator::deserialize_report
pub(crate) struct ReportSeed<'a> {
    pub(crate) registry: &'a GroupRegistry,
}

impl<'de> serde::de::DeserializeSeed<'de> for ReportSeed<'_> {
    type Value = Report;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Report, D::Error> {
        deserializer.deserialize_struct("Report", &["body", "parts"], self)
    }
}

impl<'de> Visitor<'de> for ReportSeed<'_> {
    type Value = Report;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a { body, parts } report")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Report, A::Error> {
        let mut report = Report::new();
        let mut seen_body = false;
        let mut seen_parts = false;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "body" => {
                    if seen_body {
                        return Err(DeError::duplicate_field("body"));
                    }
                    seen_body = true;
                    if let Some((type_id, entities)) = map.next_value_seed(OptionGroupSeed {
                        registry: self.registry,
                    })? {
                        report.body = Some(BodyReport {
                            modality: type_id,
                            entities,
                        });
                    }
                }
                "parts" => {
                    if seen_parts {
                        return Err(DeError::duplicate_field("parts"));
                    }
                    seen_parts = true;
                    let parts: HashMap<String, Option<ParsedGroup>> =
                        map.next_value_seed(PartsSeed {
                            registry: self.registry,
                        })?;
                    // A `None` value is a part that was skipped (unregistered and
                    // empty); its id was reserved only to catch duplicates.
                    for (id, group) in parts {
                        let Some((type_id, entities)) = group else {
                            continue;
                        };
                        report.parts.insert(
                            PartId::from(id),
                            PartReport {
                                modality: type_id,
                                handle: None,
                                entities,
                            },
                        );
                    }
                }
                // `usage` (and any future field) is ignored: it is derived
                // analysis output, not editable review state. Both `body` and
                // `parts` are optional — a body-less document, or one with no
                // container parts, is a valid report.
                _ => {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
        }
        Ok(report)
    }
}

/// `body` is `Option<Group>`: null when no body pipeline ran.
struct OptionGroupSeed<'a> {
    registry: &'a GroupRegistry,
}

impl<'de> DeserializeSeed<'de> for OptionGroupSeed<'_> {
    type Value = Option<ParsedGroup>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_option(self)
    }
}

impl<'de> Visitor<'de> for OptionGroupSeed<'_> {
    type Value = Option<ParsedGroup>;

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
        GroupSeed {
            registry: self.registry,
        }
        .deserialize(d)
    }
}

/// `parts` is a map of `PartId` -> group.
struct PartsSeed<'a> {
    registry: &'a GroupRegistry,
}

impl<'de> DeserializeSeed<'de> for PartsSeed<'_> {
    type Value = HashMap<String, Option<ParsedGroup>>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for PartsSeed<'_> {
    type Value = HashMap<String, Option<ParsedGroup>>;

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
            let group = map.next_value_seed(GroupSeed {
                registry: self.registry,
            })?;
            out.insert(id, group);
        }
        Ok(out)
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
    fn text_registry() -> GroupRegistry {
        let mut r = GroupRegistry::default();
        r.register::<Text>();
        r
    }

    fn round_trip(report: &Report, registry: &GroupRegistry) -> Report {
        let json = serde_json::to_string(report).expect("serialize");
        let mut de = serde_json::Deserializer::from_str(&json);
        match (ReportSeed { registry }).deserialize(&mut de) {
            Ok(report) => report,
            Err(e) => panic!("deserialize: {e}"),
        }
    }

    #[test]
    fn round_trips_a_body_report() {
        let report = Report::new().insert_body::<Text>(vec![text_entity("EMAIL_ADDRESS")]);
        let mut back = round_trip(&report, &text_registry());

        let body = back.entities::<Text>().expect("body reconstructed as Text");
        assert_eq!(body.len(), 1);
        assert_eq!(body[0].label, LabelRef::new("EMAIL_ADDRESS"));
        assert!(body[0].audit.verify().is_ok(), "the audit trail survives");
    }

    #[test]
    fn round_trips_parts_keyed_by_id() {
        let part = PartId::from("word/media/image1.png".to_owned());
        let report = Report::new()
            .insert_body::<Text>(vec![text_entity("A")])
            .insert_part::<Text>(part.clone(), vec![text_entity("B")]);
        let mut back = round_trip(&report, &text_registry());

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
        let err = match (ReportSeed {
            registry: &text_registry(),
        })
        .deserialize(&mut de)
        {
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
        let back = match (ReportSeed {
            registry: &text_registry(),
        })
        .deserialize(&mut de)
        {
            Ok(report) => report,
            Err(e) => panic!("empty unregistered groups must be skipped: {e}"),
        };
        assert!(back.body.is_none(), "empty unregistered body skipped");
        assert!(back.parts.is_empty(), "empty unregistered part skipped");
    }

    /// A review layer's JSON tooling may reorder object keys, so a group with
    /// `entities` before `modality` must still parse — the reconstruction is
    /// independent of wire key order.
    #[test]
    fn group_fields_may_arrive_in_any_order() {
        // `entities` first, then `modality` — the reverse of what we emit.
        let json = r#"{"parts":{},"body":{"entities":[],"modality":"text"}}"#;
        let mut de = serde_json::Deserializer::from_str(json);
        let mut report = match (ReportSeed {
            registry: &text_registry(),
        })
        .deserialize(&mut de)
        {
            Ok(report) => report,
            Err(e) => panic!("key order must not matter: {e}"),
        };
        assert!(report.entities::<Text>().is_some(), "body reconstructed");
    }

    /// An unknown group field (a later format version) is ignored, matching the
    /// report-level policy — the wire format can grow additively.
    #[test]
    fn unknown_group_fields_are_ignored() {
        let json = r#"{"body":{"modality":"text","entities":[],"future":42},"parts":{}}"#;
        let mut de = serde_json::Deserializer::from_str(json);
        assert!(
            (ReportSeed {
                registry: &text_registry(),
            })
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
        let err = match (ReportSeed {
            registry: &text_registry(),
        })
        .deserialize(&mut de)
        {
            Ok(_) => panic!("duplicate `body` must be rejected"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("duplicate field"), "got: {err}");
    }

    /// A repeated part id is rejected: silently overwriting the earlier group
    /// would drop its (possibly reviewer-edited) entities, and `anonymize_with`
    /// would apply only the later group.
    #[test]
    fn duplicate_part_id_is_rejected() {
        let json = concat!(
            r#"{"body":null,"parts":{"#,
            r#""a/b.txt":{"modality":"text","entities":[]},"#,
            r#""a/b.txt":{"modality":"text","entities":[]}}}"#,
        );
        let mut de = serde_json::Deserializer::from_str(json);
        let err = match (ReportSeed {
            registry: &text_registry(),
        })
        .deserialize(&mut de)
        {
            Ok(_) => panic!("a duplicate part id must be rejected"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("duplicate part id `a/b.txt`"),
            "got: {err}",
        );
    }
}
