//! [`Report`]: the detected-but-not-yet-applied entities of a whole
//! document, editable per modality before [`anonymize_with`].
//!
//! Detection (`analyze`) and redaction (`anonymize_with`) are split so a
//! caller can inspect and edit the entities in between — drop a
//! false-positive, retag, retarget a span. A document's entities span
//! several coordinate systems (the body's modality, plus each container
//! part's), so the report keeps them separated: the body entities keyed by
//! their modality, and each part's entities keyed by the part id, each
//! editable through a typed accessor.
//!
//! With the `serde` feature the report serializes to a part-grouped view —
//! `{ body: [..], parts: { "word/media/image1.png": [..] } }` — so an
//! external consumer (a review UI) can identify which part each entity
//! belongs to. The part id is the map key; each entity carries its own id,
//! label, location, and confidence.
//!
//! [`anonymize_with`]: super::Orchestrator::anonymize_with

use std::any::{Any, TypeId};
use std::collections::HashMap;

use elide_codec::{PartId, UntypedDocumentHandle};
use elide_core::entity::Entity;
use elide_core::modality::Modality;

/// A type-erased, downcastable group of entities (a `Vec<Entity<M>>`).
///
/// An implementation detail of the report's storage, surfaced only because
/// it appears as a bound (`Vec<Entity<M>>: EntityGroup`) on the
/// orchestrator's construction methods. Lets groups of different
/// modalities sit together while each stays recoverable by downcast; under
/// the `serde` feature it is additionally erased-serializable.
#[doc(hidden)]
pub trait EntityGroup: Send + Sync + MaybeErased {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<M: Modality> EntityGroup for Vec<Entity<M>>
where
    Vec<Entity<M>>: MaybeErased,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// `MaybeErased` carries the serde-conditional capability in one place: it
// is `erased_serde::Serialize` with serde on, and a vacuous marker with it
// off. So `EntityGroup` and its construction sites need no `#[cfg]`.
#[cfg(feature = "serde")]
#[doc(hidden)]
pub use erased_serde::Serialize as MaybeErased;

#[cfg(not(feature = "serde"))]
#[doc(hidden)]
pub trait MaybeErased {}
#[cfg(not(feature = "serde"))]
impl<T> MaybeErased for T {}

/// One container part captured during analysis: its detected entities, the
/// modality they belong to, and — for the same-process fast path — the
/// decoded part handle.
pub(super) struct PartReport {
    /// The part's modality, the routing key for [`anonymize_with`]: it
    /// re-fetches the part from the container and applies through the
    /// pipeline registered for this modality.
    ///
    /// [`anonymize_with`]: super::Orchestrator::anonymize_with
    pub(super) modality: TypeId,
    /// The decoded part handle, retained from analysis as a same-process
    /// cache. `Some` after [`analyze`] (so apply re-drives it directly with
    /// no second decode); `None` for a [`Report`] built by hand or rebuilt
    /// from serialized entities, where apply re-decodes the part from the
    /// container instead.
    ///
    /// Never serialized — a live decoded document is not data.
    ///
    /// [`analyze`]: super::Orchestrator::analyze
    pub(super) handle: Option<UntypedDocumentHandle>,
    /// The part's detected entities (a `Vec<Entity<P>>`).
    pub(super) entities: Box<dyn EntityGroup>,
}

/// The detected entities of a whole document, editable before apply.
///
/// Returned by [`analyze`] and consumed by [`anonymize_with`]. Edit the
/// body entities of modality `M` with [`entities`], and a part's with
/// [`part_entities`]; both hand back a `&mut Vec<Entity<_>>` you can
/// filter, retag, or extend before applying.
///
/// A report is **pure entity data** — it carries no live document state, so
/// it can be built from scratch ([`new`] + [`insert_body`] /
/// [`insert_part`]) and, with the `serde` feature, serialized to a
/// part-grouped `{ body, parts }` view, shipped elsewhere, and reconstructed
/// there. To round-trip: serialize a report, edit the JSON, deserialize each
/// group back into a `Vec<Entity<M>>` (the caller knows the modality), and
/// rebuild with [`new`] + the `insert_*` methods. [`anonymize_with`] then
/// re-decodes each part from the container it is applied to, so a rebuilt
/// report redacts just as a freshly-analyzed one does.
///
/// [`analyze`]: super::Orchestrator::analyze
/// [`anonymize_with`]: super::Orchestrator::anonymize_with
/// [`entities`]: Report::entities
/// [`part_entities`]: Report::part_entities
/// [`new`]: Report::new
/// [`insert_body`]: Report::insert_body
/// [`insert_part`]: Report::insert_part
#[derive(Default)]
pub struct Report {
    /// The body's entities keyed by their modality's `TypeId`. A document
    /// has exactly one body modality, so this holds at most one entry.
    pub(super) body: Option<(TypeId, Box<dyn EntityGroup>)>,
    /// Each container part's entry, keyed by its [`PartId`].
    pub(super) parts: HashMap<PartId, PartReport>,
}

impl Report {
    /// An empty report — no body, no parts. Fill it with [`insert_body`]
    /// and [`insert_part`], or let [`analyze`] produce one.
    ///
    /// [`insert_body`]: Self::insert_body
    /// [`insert_part`]: Self::insert_part
    /// [`analyze`]: super::Orchestrator::analyze
    pub fn new() -> Self {
        Self {
            body: None,
            parts: HashMap::new(),
        }
    }

    /// Set the body entities of modality `M`, replacing any already set.
    ///
    /// For rebuilding a report from out-of-band entities (e.g. deserialized
    /// from a review tool). [`anonymize_with`] reads these back through the
    /// `M` pipeline.
    ///
    /// [`anonymize_with`]: super::Orchestrator::anonymize_with
    #[must_use]
    pub fn insert_body<M: Modality>(mut self, entities: Vec<Entity<M>>) -> Self
    where
        Vec<Entity<M>>: EntityGroup,
    {
        self.body = Some((TypeId::of::<M>(), Box::new(entities)));
        self
    }

    /// Set the entities of the container part `id`, as modality `P`,
    /// replacing any already set for that part.
    ///
    /// For rebuilding a report from out-of-band entities. [`anonymize_with`]
    /// re-decodes the part `id` from the container and applies these through
    /// the `P` pipeline.
    ///
    /// [`anonymize_with`]: super::Orchestrator::anonymize_with
    #[must_use]
    pub fn insert_part<P: Modality>(mut self, id: PartId, entities: Vec<Entity<P>>) -> Self
    where
        Vec<Entity<P>>: EntityGroup,
    {
        self.parts.insert(
            id,
            PartReport {
                modality: TypeId::of::<P>(),
                handle: None,
                entities: Box::new(entities),
            },
        );
        self
    }

    /// The body entities of modality `M`, for inspection or editing.
    /// Returns `None` if the document's body is a different modality (or
    /// no body pipeline ran).
    pub fn entities<M: Modality>(&mut self) -> Option<&mut Vec<Entity<M>>> {
        let (type_id, boxed) = self.body.as_mut()?;
        if *type_id != TypeId::of::<M>() {
            return None;
        }
        boxed.as_any_mut().downcast_mut::<Vec<Entity<M>>>()
    }

    /// The entities of the container part identified by `id`, as modality
    /// `P`, for inspection or editing. Returns `None` for an unknown part or
    /// a modality mismatch.
    pub fn part_entities<P: Modality>(&mut self, id: &PartId) -> Option<&mut Vec<Entity<P>>> {
        let part = self.parts.get_mut(id)?;
        if part.modality != TypeId::of::<P>() {
            return None;
        }
        part.entities.as_any_mut().downcast_mut::<Vec<Entity<P>>>()
    }

    /// The [`PartId`]s of the container parts the report carries, paired
    /// with each part's modality `TypeId` — for a caller enumerating what's
    /// editable.
    pub fn part_ids(&self) -> impl Iterator<Item = (&PartId, TypeId)> {
        self.parts.iter().map(|(id, p)| (id, p.modality))
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Report {
    /// Serialize to `{ body: [entities], parts: { id: [entities] } }`.
    /// `body` is null when no body pipeline ran.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        // Adapt an erased group to a Serialize value.
        struct Group<'a>(&'a dyn EntityGroup);
        impl serde::Serialize for Group<'_> {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                erased_serde::serialize(self.0, s)
            }
        }

        let parts: HashMap<&str, Group<'_>> = self
            .parts
            .iter()
            .map(|(id, p)| (id.as_str(), Group(p.entities.as_ref())))
            .collect();

        let mut state = serializer.serialize_struct("Report", 2)?;
        state.serialize_field("body", &self.body.as_ref().map(|(_, g)| Group(g.as_ref())))?;
        state.serialize_field("parts", &parts)?;
        state.end()
    }
}

#[cfg(all(test, feature = "serde"))]
mod tests {
    use elide_core::entity::provenance::{Event, PatternEvent, Provenance};
    use elide_core::entity::{Entity, LabelRef};
    use elide_core::modality::text::{Text, TextLocation};
    use elide_core::primitive::Confidence;

    use super::*;

    fn text_entity(label: &str) -> Entity<Text> {
        let loc = TextLocation::new(0, 4);
        let event = Event::pattern("t", Confidence::MAX, loc.clone(), PatternEvent::default());
        Entity::new(
            LabelRef::new(label),
            loc,
            Confidence::MAX,
            Provenance::new(event),
        )
    }

    #[test]
    fn serializes_body_to_grouped_view() {
        // The part-grouped `{ body, parts }` shape is exercised end to end
        // (with a real container) in the docx integration test; here we
        // check the body group and the empty-parts shape directly.
        let report = Report::new().insert_body::<Text>(vec![text_entity("EMAIL_ADDRESS")]);

        let value = serde_json::to_value(&report).unwrap();
        // body is an array carrying the entity's label; parts is an object.
        assert_eq!(value["body"][0]["label"], "EMAIL_ADDRESS");
        assert!(value["parts"].is_object());
        assert_eq!(value["parts"].as_object().unwrap().len(), 0);

        // No body pipeline ran → body is null.
        let empty = serde_json::to_value(Report::new()).unwrap();
        assert!(empty["body"].is_null());
    }
}
