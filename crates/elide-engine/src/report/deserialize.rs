//! Reconstructing a [`Report`] from its serialized wire form, driven by the
//! orchestrator's registered modalities.
//!
//! The wire form drops the concrete modality type — a group is just tagged with
//! its [`Modality::NAME`]. Deserialization is not object-safe, so the report
//! cannot recover the type on its own. Instead the [`Orchestrator`] holds a
//! [`GroupRegistry`]: [`with_modality`] registers, per modality name, a parser
//! that deserializes that group as the concrete `Vec<Entity<M>>`. A group whose
//! tag names no registered modality is a hard error — the report would silently
//! lose those entities (and any reviewer edits on them) otherwise.
//!
//! [`Report`]: super::Report
//! [`Orchestrator`]: crate::Orchestrator
//! [`with_modality`]: crate::Orchestrator::with_modality
//! [`Modality::NAME`]: elide_core::modality::Modality::NAME

use std::any::TypeId;
use std::collections::HashMap;

use elide_core::entity::Entity;
use elide_core::modality::Modality;
use serde::de::{DeserializeSeed, Deserializer, Error as DeError, MapAccess, Visitor};

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

    /// The entry registered for `modality`, or a [`DeError`] naming the missing
    /// modality — a report referencing an unregistered modality is rejected
    /// rather than silently dropping its entities.
    fn entry<E: DeError>(&self, modality: &str) -> Result<ModalityEntry, E> {
        self.entries
            .get(modality)
            .copied()
            .ok_or_else(|| DeError::custom(format!("no registered modality for `{modality}`")))
    }
}

/// A parsed group with the routing [`TypeId`] its modality registered.
type ParsedGroup = (TypeId, Box<dyn EntityGroup>);

/// A serialized group envelope: `{ modality, entities }`. Deserializes by
/// reading the `modality` tag first, resolving its registered entry, then
/// running that entry's parser over `entities`.
struct GroupSeed<'a> {
    registry: &'a GroupRegistry,
}

impl<'de> DeserializeSeed<'de> for GroupSeed<'_> {
    type Value = ParsedGroup;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_struct("Group", &["modality", "entities"], self)
    }
}

impl<'de> Visitor<'de> for GroupSeed<'_> {
    type Value = ParsedGroup;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a { modality, entities } group")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut entry: Option<ModalityEntry> = None;
        let mut group: Option<Box<dyn EntityGroup>> = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "modality" => {
                    if entry.is_some() {
                        return Err(DeError::duplicate_field("modality"));
                    }
                    let name: String = map.next_value()?;
                    entry = Some(self.registry.entry(&name)?);
                }
                "entities" => {
                    // `modality` precedes `entities` on the wire (our serializer
                    // emits them in that order), so the parser is resolved by
                    // the time the entities are read.
                    let resolved = entry
                        .ok_or_else(|| DeError::custom("`modality` must precede `entities`"))?;
                    group = Some(map.next_value_seed(ParseWith(resolved.parse))?);
                }
                other => return Err(DeError::unknown_field(other, &["modality", "entities"])),
            }
        }
        let entry = entry.ok_or_else(|| DeError::missing_field("modality"))?;
        let group = group.ok_or_else(|| DeError::missing_field("entities"))?;
        Ok((entry.type_id, group))
    }
}

/// A `DeserializeSeed` that runs a registered [`GroupParser`] over the entities
/// value, bridging serde's `Deserializer` to `erased_serde`.
struct ParseWith(GroupParser);

impl<'de> serde::de::DeserializeSeed<'de> for ParseWith {
    type Value = Box<dyn EntityGroup>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        let mut erased = <dyn erased_serde::Deserializer<'_>>::erase(deserializer);
        (self.0)(&mut erased).map_err(DeError::custom)
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
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "body" => {
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
                    let parts: HashMap<String, ParsedGroup> = map.next_value_seed(PartsSeed {
                        registry: self.registry,
                    })?;
                    for (id, (type_id, entities)) in parts {
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
                // analysis output, not editable review state.
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
        GroupSeed {
            registry: self.registry,
        }
        .deserialize(d)
        .map(Some)
    }
}

/// `parts` is a map of `PartId` -> group.
struct PartsSeed<'a> {
    registry: &'a GroupRegistry,
}

impl<'de> DeserializeSeed<'de> for PartsSeed<'_> {
    type Value = HashMap<String, ParsedGroup>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> Visitor<'de> for PartsSeed<'_> {
    type Value = HashMap<String, ParsedGroup>;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a map of part id to group")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut out = HashMap::new();
        while let Some(id) = map.next_key::<String>()? {
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

    /// A group naming a modality the registry does not know is rejected, rather
    /// than silently dropping its entities.
    #[test]
    fn unregistered_modality_is_rejected() {
        // A body group tagged with a modality the registry has no parser for.
        let json = r#"{"body":{"modality":"audio","entities":[]},"parts":{}}"#;
        let mut de = serde_json::Deserializer::from_str(json);
        let err = match (ReportSeed {
            registry: &text_registry(),
        })
        .deserialize(&mut de)
        {
            Ok(_) => panic!("unregistered modality must be rejected"),
            Err(e) => e,
        };
        assert!(
            err.to_string()
                .contains("no registered modality for `audio`"),
            "got: {err}",
        );
    }
}
