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
//! The type-erased storage ([`EntityGroup`] / [`SelectionGroup`]) lives in
//! [`group`], the per-group report entries ([`BodyReport`] / [`PartReport`])
//! in [`entry`], and the serde wire view in `serialize`.
//!
//! [`BodyReport`]: entry::BodyReport
//! [`PartReport`]: entry::PartReport
//! [`anonymize_with`]: super::Orchestrator::anonymize_with

mod entry;
mod group;
#[cfg(feature = "serde")]
mod serialize;

use std::any::TypeId;
use std::collections::HashMap;
use std::ops::ControlFlow;

use elide_codec::PartId;
use elide_core::entity::Entity;
use elide_core::modality::Modality;
use uuid::Uuid;

pub(crate) use self::entry::{BodyReport, PartReport};
pub use self::group::{EntityGroup, SelectionGroup};

/// The detected entities of a whole document, editable before apply.
///
/// Returned by [`analyze`] and consumed by [`anonymize_with`]. Edit the
/// body entities of modality `M` with [`entities`], and a part's with
/// [`part_entities`]; both hand back a `&mut Vec<Entity<_>>` you can
/// filter, retag, or extend before applying. To reach one entity by
/// [`id`] use [`entity_mut`] / [`part_entity_mut`]; to walk a whole group
/// in a single mutable pass (e.g. merging the applied report's provenance
/// back onto a caller's records) use [`for_each_body_mut`] /
/// [`for_each_part_mut`], or their `try_` variants to stop the walk early.
///
/// [`id`]: elide_core::entity::Entity::id
/// [`entity_mut`]: Report::entity_mut
/// [`part_entity_mut`]: Report::part_entity_mut
/// [`for_each_body_mut`]: Report::for_each_body_mut
/// [`for_each_part_mut`]: Report::for_each_part_mut
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
    /// The document body's entry, if a body pipeline ran. A document has
    /// exactly one body, so this holds at most one [`BodyReport`].
    pub(crate) body: Option<BodyReport>,
    /// Each container part's entry, keyed by its [`PartId`].
    pub(crate) parts: HashMap<PartId, PartReport>,
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
        self.body = Some(BodyReport {
            modality: TypeId::of::<M>(),
            entities: Box::new(entities),
        });
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
        let body = self.body.as_mut()?;
        if body.modality != TypeId::of::<M>() {
            return None;
        }
        body.entities.as_any_mut().downcast_mut::<Vec<Entity<M>>>()
    }

    /// One body entity of modality `M`, by its [`id`]. Returns `None` if the
    /// body is a different modality, no body pipeline ran, or no entity has
    /// that id.
    ///
    /// The id-addressed counterpart to [`entities`], for a caller that holds
    /// an entity's id (from the analyzed report) and wants to reach the same
    /// entity in the applied report — e.g. to merge its post-redaction
    /// provenance back onto its own record without scanning the whole group.
    ///
    /// [`id`]: elide_core::entity::Entity::id
    /// [`entities`]: Self::entities
    pub fn entity_mut<M: Modality>(&mut self, id: Uuid) -> Option<&mut Entity<M>> {
        self.entities::<M>()?.iter_mut().find(|e| e.id == id)
    }

    /// Run `f` over every body entity of modality `M`, in one pass. A no-op
    /// when the body is a different modality or no body pipeline ran.
    ///
    /// The batch counterpart to [`entity_mut`]: a caller merging the applied
    /// report's per-entity provenance back onto its own records walks the
    /// group once here — keyed by [`id`] inside `f` — instead of an
    /// id-lookup per record. One linear pass, no per-call dispatch.
    ///
    /// [`entity_mut`]: Self::entity_mut
    /// [`id`]: elide_core::entity::Entity::id
    pub fn for_each_body_mut<M: Modality>(&mut self, f: impl FnMut(&mut Entity<M>)) {
        if let Some(entities) = self.entities::<M>() {
            entities.iter_mut().for_each(f);
        }
    }

    /// Like [`for_each_body_mut`], but `f` returns a [`ControlFlow`] so the
    /// walk can stop early — [`ControlFlow::Break`] halts and returns its
    /// value, [`ControlFlow::Continue`] proceeds. Returns
    /// `ControlFlow::Continue(())` when the walk ran to the end, and also
    /// (vacuously) when the body is a different modality or no body pipeline
    /// ran.
    ///
    /// For a caller that stops once it has done its work — e.g. merged every
    /// record it holds — rather than always traversing the whole group.
    ///
    /// [`for_each_body_mut`]: Self::for_each_body_mut
    // Returns a concrete `ControlFlow<B>` rather than being generic over
    // `Try`, which is nightly-only (`try_trait_v2`); revisit when it
    // stabilizes — see issue #139.
    pub fn try_for_each_body_mut<M: Modality, B>(
        &mut self,
        f: impl FnMut(&mut Entity<M>) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        match self.entities::<M>() {
            Some(entities) => entities.iter_mut().try_for_each(f),
            None => ControlFlow::Continue(()),
        }
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

    /// One entity of the container part `part_id`, as modality `P`, by its
    /// [`id`]. Returns `None` for an unknown part, a modality mismatch, or no
    /// entity with that id.
    ///
    /// The id-addressed counterpart to [`part_entities`].
    ///
    /// [`id`]: elide_core::entity::Entity::id
    /// [`part_entities`]: Self::part_entities
    pub fn part_entity_mut<P: Modality>(
        &mut self,
        part_id: &PartId,
        id: Uuid,
    ) -> Option<&mut Entity<P>> {
        self.part_entities::<P>(part_id)?
            .iter_mut()
            .find(|e| e.id == id)
    }

    /// Run `f` over every entity of the container part `part_id`, as modality
    /// `P`, in one pass. A no-op for an unknown part or a modality mismatch.
    ///
    /// The batch counterpart to [`part_entity_mut`], mirroring
    /// [`for_each_body_mut`] for a part.
    ///
    /// [`part_entity_mut`]: Self::part_entity_mut
    /// [`for_each_body_mut`]: Self::for_each_body_mut
    pub fn for_each_part_mut<P: Modality>(
        &mut self,
        part_id: &PartId,
        f: impl FnMut(&mut Entity<P>),
    ) {
        if let Some(entities) = self.part_entities::<P>(part_id) {
            entities.iter_mut().for_each(f);
        }
    }

    /// Like [`for_each_part_mut`], but `f` returns a [`ControlFlow`] so the
    /// walk can stop early. The part counterpart to [`try_for_each_body_mut`];
    /// returns `ControlFlow::Continue(())` on a full walk, an unknown part, or
    /// a modality mismatch.
    ///
    /// [`for_each_part_mut`]: Self::for_each_part_mut
    /// [`try_for_each_body_mut`]: Self::try_for_each_body_mut
    pub fn try_for_each_part_mut<P: Modality, B>(
        &mut self,
        part_id: &PartId,
        f: impl FnMut(&mut Entity<P>) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        match self.part_entities::<P>(part_id) {
            Some(entities) => entities.iter_mut().try_for_each(f),
            None => ControlFlow::Continue(()),
        }
    }

    /// The [`PartId`]s of the container parts the report carries, paired
    /// with each part's modality [`TypeId`] — for a caller enumerating what's
    /// editable.
    pub fn part_ids(&self) -> impl Iterator<Item = (&PartId, TypeId)> {
        self.parts.iter().map(|(id, p)| (id, p.modality))
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::provenance::{Event, PatternEvent, Provenance};
    use elide_core::entity::{Entity, LabelRef};
    use elide_core::modality::text::{Text, TextLocation};
    use elide_core::primitive::Confidence;

    use super::*;

    /// A minimal text entity carrying `label`, for building reports under test.
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
    fn entity_mut_addresses_body_by_id() {
        let a = text_entity("EMAIL_ADDRESS");
        let b = text_entity("PHONE_NUMBER");
        let (id_a, id_b) = (a.id, b.id);
        let mut report = Report::new().insert_body::<Text>(vec![a, b]);

        // A present id resolves to that exact entity.
        assert_eq!(
            report.entity_mut::<Text>(id_b).map(|e| e.label.as_str()),
            Some("PHONE_NUMBER")
        );
        assert_eq!(
            report.entity_mut::<Text>(id_a).map(|e| e.label.as_str()),
            Some("EMAIL_ADDRESS")
        );
        // An unknown id misses. (Modality-mismatch and no-body-pipeline both
        // route through `entities`, which is covered by its own tests.)
        assert!(report.entity_mut::<Text>(Uuid::now_v7()).is_none());
    }

    #[test]
    fn part_entity_mut_addresses_a_part_by_id() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id;
        let part = PartId::new("word/media/image1.png");
        let mut report = Report::new().insert_part::<Text>(part.clone(), vec![entity]);

        assert_eq!(
            report
                .part_entity_mut::<Text>(&part, id)
                .map(|e| e.label.as_str()),
            Some("EMAIL_ADDRESS")
        );
        // An unknown part and an unknown id both miss.
        assert!(
            report
                .part_entity_mut::<Text>(&PartId::new("nope"), id)
                .is_none()
        );
        assert!(report.part_entity_mut::<Text>(&part, Uuid::now_v7()).is_none());
    }

    #[test]
    fn for_each_body_mut_visits_every_entity_once() {
        let mut report = Report::new().insert_body::<Text>(vec![
            text_entity("EMAIL_ADDRESS"),
            text_entity("PHONE_NUMBER"),
        ]);

        // The pass mutates in place and visits each entity exactly once.
        let mut count = 0;
        report.for_each_body_mut::<Text>(|e| {
            count += 1;
            e.label = LabelRef::new("REDACTED");
        });
        assert_eq!(count, 2);
        assert!(
            report
                .entities::<Text>()
                .unwrap()
                .iter()
                .all(|e| e.label.as_str() == "REDACTED")
        );

        // No body pipeline → the closure never runs.
        let mut ran = false;
        Report::new().for_each_body_mut::<Text>(|_| ran = true);
        assert!(!ran);
    }

    #[test]
    fn try_for_each_body_mut_breaks_early() {
        let a = text_entity("EMAIL_ADDRESS");
        let target = a.id;
        let mut report = Report::new().insert_body::<Text>(vec![a, text_entity("PHONE_NUMBER")]);

        // Break carries a value out and halts the walk at the first match.
        let mut visited = 0;
        let found = report.try_for_each_body_mut::<Text, &'static str>(|e| {
            visited += 1;
            if e.id == target {
                ControlFlow::Break("hit")
            } else {
                ControlFlow::Continue(())
            }
        });
        assert_eq!(found, ControlFlow::Break("hit"));
        assert_eq!(visited, 1, "the walk stops at the first break");

        // A full walk (never breaking) returns Continue.
        let done = report.try_for_each_body_mut::<Text, ()>(|_| ControlFlow::Continue(()));
        assert_eq!(done, ControlFlow::Continue(()));

        // No body pipeline → vacuously Continue.
        let empty = Report::new().try_for_each_body_mut::<Text, ()>(|_| ControlFlow::Break(()));
        assert_eq!(empty, ControlFlow::Continue(()));
    }

    #[test]
    fn for_each_part_mut_visits_a_part() {
        let part = PartId::new("word/media/image1.png");
        let mut report =
            Report::new().insert_part::<Text>(part.clone(), vec![text_entity("EMAIL_ADDRESS")]);

        let mut count = 0;
        report.for_each_part_mut::<Text>(&part, |_| count += 1);
        assert_eq!(count, 1);

        // An unknown part → the closure never runs.
        let mut ran = false;
        report.for_each_part_mut::<Text>(&PartId::new("nope"), |_| ran = true);
        assert!(!ran);
    }
}
