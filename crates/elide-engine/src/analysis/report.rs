//! [`Report`]: the detected-but-not-yet-applied entities of a document set,
//! editable per part before [`anonymize_with`].
//!
//! Detection (`analyze`) and redaction (`anonymize_with`) are split so a
//! caller can inspect and edit the entities in between, drop a
//! false-positive, retag, retarget a span. A document set's entities span
//! several coordinate systems (each named document's own modality, plus every
//! container part's), so the report keeps them separated: every part's entities
//! are keyed by its [`PartId`] path, each editable through a typed accessor.
//! There is no distinct "body", a document's own content is simply its
//! depth-1 part, keyed by the document's name.
//!
//! With the `serde` feature the report serializes to a part-list view,
//! `{ parts: [ { "id": ["scan-A.docx", "word/media/image1.png"], .. } ] }`,
//! so an external consumer (a review UI) can identify which part each entity
//! belongs to. The path is a segment array (never string-joined); each entity
//! carries its own id, label, location, and confidence.
//!
//! The type-erased storage ([`EntityGroup`]) lives in [`group`](super::group),
//! and the serde wire view in [`serde`](super::serde). Each entity carries its
//! own tamper-evident audit trail ([`AuditLog`]) natively, so there is no
//! separate document-level audit type.
//!
//! [`AuditLog`]: elide_core::entity::audit::AuditLog
//! [`anonymize_with`]: crate::Orchestrator::anonymize_with

use std::any::TypeId;
use std::collections::HashMap;
use std::ops::ControlFlow;

use elide_codec::UntypedDocumentHandle;
use elide_core::entity::audit::{Attribution, AuditEvent, AuditKind, ManualIntent};
use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::Modality;
#[cfg(feature = "usage")]
use elide_core::primitive::UsageReport;
use uuid::Uuid;

use super::group::EntityGroup;
use super::registry::ReportDeserializer;
use crate::PartId;

/// One part captured during analysis, a named document's own content (a
/// depth-1 part) or a container part nested within one: its detected entities,
/// the modality they belong to, and, for the same-process fast path, the
/// decoded part handle.
pub(crate) struct PartReport {
    /// The part's modality, the routing key for [`anonymize_with`]: it
    /// re-fetches the part from the container and applies through the
    /// pipeline registered for this modality.
    ///
    /// [`anonymize_with`]: crate::Orchestrator::anonymize_with
    pub(crate) modality: TypeId,
    /// The decoded part handle, retained from analysis as a same-process
    /// cache. `Some` after [`analyze`] (so apply re-drives it directly with
    /// no second decode); `None` for a [`Report`] built by hand or rebuilt
    /// from serialized entities, where apply re-decodes the part from the
    /// container instead.
    ///
    /// Never serialized, a live decoded document is not data.
    ///
    /// [`analyze`]: crate::Orchestrator::analyze
    pub(crate) handle: Option<UntypedDocumentHandle>,
    /// The part's detected entities (a `Vec<Entity<P>>`).
    pub(crate) entities: Box<dyn EntityGroup>,
}

/// The detected entities of a document set, editable before apply.
///
/// Returned by [`analyze`] and consumed by [`anonymize_with`]. Every part,
/// each named document's own content, and every container part within one, is
/// keyed by its [`PartId`] path. Read a part's entities of modality `P` with
/// [`part_entities`], returning a `&[Entity<_>]`; edit them with
/// [`part_entities_mut`], returning a `&mut Vec<Entity<_>>` you can filter,
/// retag, or extend before applying. When the report describes a **single**
/// document, [`entities`] / [`entities_mut`] are a shorthand for its sole
/// content (see below). To reach one entity by [`id`] use [`part_entity`] /
/// [`part_entity_mut`] when the part is known, or [`entity_anywhere`] /
/// [`entity_anywhere_mut`] when a review layer holds only the id and not the
/// part it lives in. To walk a whole group in a single mutable pass (e.g. merging
/// the applied report's provenance back onto a caller's records) use
/// [`for_each_part_mut`], or its `try_` variant to stop the walk early.
///
/// The single-document shorthand: [`entities`] / [`entities_mut`] /
/// [`entity`] / [`entity_mut`] read the *sole* document's content, the one
/// depth-1 part, and return `None` when the report holds zero or more than one
/// top-level document (a multi-document set: use [`part_entities`] with the
/// document's name).
///
/// [`id`]: elide_core::entity::Entity::id
/// [`entities`]: Report::entities
/// [`entities_mut`]: Report::entities_mut
/// [`entity`]: Report::entity
/// [`entity_mut`]: Report::entity_mut
/// [`part_entity`]: Report::part_entity
/// [`part_entity_mut`]: Report::part_entity_mut
/// [`entity_anywhere`]: Report::entity_anywhere
/// [`entity_anywhere_mut`]: Report::entity_anywhere_mut
/// [`for_each_part_mut`]: Report::for_each_part_mut
///
/// A report is **pure entity data**: it carries no live document state, so
/// it can be built from scratch ([`new`] + [`insert_part`]) and, with the
/// `serde` feature, serialized to a part-list `{ parts: [..] }` view, shipped
/// elsewhere, and reconstructed there. To round-trip: serialize a report, edit
/// the JSON, deserialize each group back into a `Vec<Entity<M>>` (the caller
/// knows the modality), and rebuild with [`new`] + [`insert_part`].
/// [`anonymize_with`] then re-decodes each part from the container it is applied
/// to, so a rebuilt report redacts just as a freshly-analyzed one does.
///
/// [`analyze`]: crate::Orchestrator::analyze
/// [`anonymize_with`]: crate::Orchestrator::anonymize_with
/// [`part_entities`]: Report::part_entities
/// [`part_entities_mut`]: Report::part_entities_mut
/// [`new`]: Report::new
/// [`insert_part`]: Report::insert_part
#[derive(Default)]
pub struct Report {
    /// Every part's entry, keyed by its [`PartId`] path, a named document's own
    /// content is its depth-1 part, container parts nest below.
    pub(crate) parts: HashMap<PartId, PartReport>,
    /// Per-recognizer / per-enricher resource usage across the whole
    /// analysis (every part), in run order.
    #[cfg(feature = "usage")]
    pub(crate) usage: UsageReport,
}

impl Report {
    /// An empty report, no parts. Fill it with [`insert_part`], or let
    /// [`analyze`] produce one.
    ///
    /// [`insert_part`]: Self::insert_part
    /// [`analyze`]: crate::Orchestrator::analyze
    pub fn new() -> Self {
        Self {
            parts: HashMap::new(),
            #[cfg(feature = "usage")]
            usage: UsageReport::new(),
        }
    }

    /// A [`ReportDeserializer`] for reconstructing a serialized report, without
    /// the analyzers, anonymizers, or codec registry an [`Orchestrator`] needs
    /// to *run* one. Register the modalities the report may contain, then
    /// [`deserialize`] it:
    ///
    /// ```ignore
    /// let report = Report::deserializer()
    ///     .with_modality::<Text>()
    ///     .deserialize(&mut serde_json::Deserializer::from_str(json))?;
    /// ```
    ///
    /// [`Orchestrator`]: crate::Orchestrator
    /// [`deserialize`]: ReportDeserializer::deserialize
    #[must_use]
    pub fn deserializer() -> ReportDeserializer {
        ReportDeserializer::new()
    }

    /// The resource usage recorded across this analysis, one entry per
    /// recognizer and enricher that ran, each self-identifying via its id.
    #[cfg(feature = "usage")]
    #[must_use]
    pub fn usage(&self) -> &UsageReport {
        &self.usage
    }

    /// Set the entities of the container part `id`, as modality `P`,
    /// replacing any already set for that part.
    ///
    /// For rebuilding a report from out-of-band entities. [`anonymize_with`]
    /// re-decodes the part `id` from the container and applies these through
    /// the `P` pipeline.
    ///
    /// [`anonymize_with`]: crate::Orchestrator::anonymize_with
    #[must_use]
    pub fn insert_part<P: Modality>(mut self, id: PartId, entities: Vec<Entity<P>>) -> Self
    where
        Vec<Entity<P>>: serde::Serialize,
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

    /// The **sole document's** entities of modality `M`, read-only, a
    /// shorthand for the single-document case. Returns `None` when the report
    /// holds zero or more than one top-level document, or the sole document is a
    /// different modality than `M`. For a multi-document set use
    /// [`part_entities`] with the document's name. Use [`entities_mut`] to edit.
    ///
    /// [`part_entities`]: Self::part_entities
    /// [`entities_mut`]: Self::entities_mut
    pub fn entities<M: Modality>(&self) -> Option<&[Entity<M>]> {
        let part = self.sole_document()?;
        if part.modality != TypeId::of::<M>() {
            return None;
        }
        part.entities
            .as_any()
            .downcast_ref::<Vec<Entity<M>>>()
            .map(Vec::as_slice)
    }

    /// The **sole document's** entities of modality `M`, for editing, the
    /// `&mut` counterpart to [`entities`]. Returns `None` when the report holds
    /// zero or more than one top-level document, or the sole document is a
    /// different modality than `M`.
    ///
    /// [`entities`]: Self::entities
    pub fn entities_mut<M: Modality>(&mut self) -> Option<&mut Vec<Entity<M>>> {
        let key = self.sole_document_id()?;
        self.part_entities_mut::<M>(&key)
    }

    /// One entity of the **sole document**, of modality `M`, by its [`id`],
    /// read-only. Returns `None` when the report holds zero or more than one
    /// top-level document, the sole document is a different modality than `M`,
    /// or no entity has that id.
    ///
    /// The id-addressed counterpart to [`entities`], for a caller that holds an
    /// entity's id (from the analyzed report) and wants to reach the same entity
    /// in the applied report, e.g. to merge its post-redaction provenance back
    /// onto its own record without scanning the whole group.
    ///
    /// [`id`]: elide_core::entity::Entity::id
    /// [`entities`]: Self::entities
    pub fn entity<M: Modality>(&self, id: Uuid) -> Option<&Entity<M>> {
        self.entities::<M>()?.iter().find(|e| e.id == id)
    }

    /// One entity of the **sole document**, of modality `M`, by its [`id`], for
    /// editing, the `&mut` counterpart to [`entity`]. Returns `None` when the
    /// report holds zero or more than one top-level document, the sole document
    /// is a different modality than `M`, or no entity has that `id`.
    ///
    /// [`id`]: elide_core::entity::Entity::id
    /// [`entity`]: Self::entity
    pub fn entity_mut<M: Modality>(&mut self, id: Uuid) -> Option<&mut Entity<M>> {
        self.entities_mut::<M>()?.iter_mut().find(|e| e.id == id)
    }

    /// Find an entity of modality `M` by its [`id`] across **every** part,
    /// read-only. For a caller that holds an entity id but not the part it lives
    /// in, a reviewer whose edit addresses an entity by id alone (a
    /// retag/suppress carries the id, not the part it was found in).
    ///
    /// Unlike [`entity`], this does not assume a single document: it scans every
    /// part, so it resolves an entity inside a nested document (a DOCX's embedded
    /// image) just as well as one in a single-file report. Parts of a modality
    /// other than `M` are skipped. Returns `None` when no part holds an entity of
    /// modality `M` with that `id`.
    ///
    /// [`id`]: elide_core::entity::Entity::id
    /// [`entity`]: Self::entity
    pub fn entity_anywhere<M: Modality>(&self, id: Uuid) -> Option<&Entity<M>> {
        self.parts
            .values()
            .filter(|part| part.modality == TypeId::of::<M>())
            .find_map(|part| {
                part.entities
                    .as_any()
                    .downcast_ref::<Vec<Entity<M>>>()?
                    .iter()
                    .find(|e| e.id == id)
            })
    }

    /// Find an entity of modality `M` by its [`id`] across **every** part, for
    /// editing, the `&mut` counterpart to [`entity_anywhere`]. For a reviewer
    /// acting on an entity by id alone (retag, suppress) without tracking which
    /// part it lives in.
    ///
    /// Unlike [`entity_mut`] (which addresses the sole document and returns
    /// `None` for a multi-part report), this reaches an entity in any part, so a
    /// reviewer edit applies to a nested document's entity too. Parts of a
    /// modality other than `M` are skipped. Returns `None` when no part holds an
    /// entity of modality `M` with that `id`.
    ///
    /// [`id`]: elide_core::entity::Entity::id
    /// [`entity_anywhere`]: Self::entity_anywhere
    /// [`entity_mut`]: Self::entity_mut
    pub fn entity_anywhere_mut<M: Modality>(&mut self, id: Uuid) -> Option<&mut Entity<M>> {
        self.parts
            .values_mut()
            .filter(|part| part.modality == TypeId::of::<M>())
            .find_map(|part| {
                part.entities
                    .as_any_mut()
                    .downcast_mut::<Vec<Entity<M>>>()?
                    .iter_mut()
                    .find(|e| e.id == id)
            })
    }

    /// Manually add `entity` to the container part `part_id`, recording a
    /// [`Manual`] event onto `entity` (unless it already carries one) so its
    /// human origin is auditable: a reviewer including a detection the engine
    /// missed is never mistaken for an automatic one. Returns `false` for an
    /// unknown part or a modality mismatch, [`insert_part`](Self::insert_part)
    /// seeds an empty part first.
    ///
    /// [`Manual`]: elide_core::entity::audit::AuditKind::Manual
    pub fn include_part<P: Modality>(&mut self, part_id: &PartId, mut entity: Entity<P>) -> bool {
        ensure_manual(&mut entity);
        match self.part_entities_mut::<P>(part_id) {
            Some(entities) => {
                entities.push(entity);
                true
            }
            None => false,
        }
    }

    /// Add a **custom** entity of modality `M` — a `label` at a `location` a
    /// reviewer marked between detection and redaction — to the **sole document**
    /// this report describes. One call: it builds the entity ([`Entity::custom`],
    /// confidence [`MAX`](elide_core::primitive::Confidence::MAX), a [`Manual`]
    /// audit event stamped) and includes it.
    ///
    /// Modality-agnostic: `location` is `M::Location`, so a custom text span,
    /// image box, audio span, or a custom modality's own coordinate all add the
    /// same way. Returns `false` when the report holds zero or more than one
    /// top-level document, or the sole document is a different modality than `M`,
    /// address a specific part with [`include_custom_at`](Self::include_custom_at)
    /// for a multi-document report.
    ///
    /// [`Manual`]: elide_core::entity::audit::AuditKind::Manual
    pub fn include_custom<M: Modality>(
        &mut self,
        label: impl Into<LabelRef>,
        location: M::Location,
    ) -> bool {
        match self.sole_document_id() {
            Some(id) => self.include_part::<M>(&id, Entity::custom(label, location)),
            None => false,
        }
    }

    /// Add a **custom** entity of modality `M` — a `label` at a `location` a
    /// reviewer marked — to the container part `part_id`. The part counterpart to
    /// [`include_custom`](Self::include_custom): builds the entity
    /// ([`Entity::custom`]) and includes it under `part_id`. Modality-agnostic in
    /// `location`. Returns `false` for an unknown part or a modality mismatch.
    pub fn include_custom_at<M: Modality>(
        &mut self,
        part_id: &PartId,
        label: impl Into<LabelRef>,
        location: M::Location,
    ) -> bool {
        self.include_part::<M>(part_id, Entity::custom(label, location))
    }

    /// Manually suppress the entity `id` in the container part `part_id`, so a
    /// reviewer can leave alone a false positive detected inside a part. Records
    /// an auditable [`Manual`] event (the *why* is `attribution`, the *who* is
    /// `actor`, recorded as the event's source). Idempotent: suppressing an
    /// already-suppressed entity records nothing (see [`Entity::record_manual`]).
    /// Returns `false` for an unknown part, a modality mismatch, or no entity
    /// with that `id`.
    ///
    /// [`Manual`]: elide_core::entity::audit::AuditKind::Manual
    /// [`Entity::record_manual`]: elide_core::entity::Entity::record_manual
    pub fn suppress_part<P: Modality>(
        &mut self,
        part_id: &PartId,
        id: Uuid,
        attribution: Option<Attribution>,
        actor: Option<String>,
    ) -> bool {
        match self.part_entity_mut::<P>(part_id, id) {
            Some(entity) => {
                entity.record_manual(ManualIntent::Suppress, attribution, actor.as_deref());
                true
            }
            None => false,
        }
    }

    /// The entities of the container part identified by `id`, as modality `P`,
    /// read-only. Returns `None` for an unknown part or a modality mismatch. Use
    /// [`part_entities_mut`] to edit them.
    ///
    /// [`part_entities_mut`]: Self::part_entities_mut
    pub fn part_entities<P: Modality>(&self, id: &PartId) -> Option<&[Entity<P>]> {
        let part = self.parts.get(id)?;
        if part.modality != TypeId::of::<P>() {
            return None;
        }
        part.entities
            .as_any()
            .downcast_ref::<Vec<Entity<P>>>()
            .map(Vec::as_slice)
    }

    /// The entities of the container part `id`, as modality `P`, for editing,
    /// the `&mut` counterpart to [`part_entities`]. Returns `None` for an
    /// unknown part or a modality mismatch.
    ///
    /// [`part_entities`]: Self::part_entities
    pub fn part_entities_mut<P: Modality>(&mut self, id: &PartId) -> Option<&mut Vec<Entity<P>>> {
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
        self.part_entities_mut::<P>(part_id)?
            .iter_mut()
            .find(|e| e.id == id)
    }

    /// One entity of the container part `part_id`, as modality `P`, by its
    /// [`id`], read-only, the `&` counterpart to [`part_entity_mut`]. Returns
    /// `None` for an unknown part, a modality mismatch, or no entity with that
    /// id.
    ///
    /// [`id`]: elide_core::entity::Entity::id
    /// [`part_entity_mut`]: Self::part_entity_mut
    pub fn part_entity<P: Modality>(&self, part_id: &PartId, id: Uuid) -> Option<&Entity<P>> {
        self.part_entities::<P>(part_id)?
            .iter()
            .find(|e| e.id == id)
    }

    /// Run `f` over every entity of the container part `part_id`, as modality
    /// `P`, in one pass. A no-op for an unknown part or a modality mismatch.
    ///
    /// The batch counterpart to [`part_entity_mut`]: a caller merging the
    /// applied report's per-entity provenance back onto its own records walks the
    /// group once here, keyed by [`id`] inside `f`, instead of an id-lookup per
    /// record. One linear pass, no per-call dispatch.
    ///
    /// [`part_entity_mut`]: Self::part_entity_mut
    /// [`id`]: elide_core::entity::Entity::id
    pub fn for_each_part_mut<P: Modality>(
        &mut self,
        part_id: &PartId,
        f: impl FnMut(&mut Entity<P>),
    ) {
        if let Some(entities) = self.part_entities_mut::<P>(part_id) {
            entities.iter_mut().for_each(f);
        }
    }

    /// Like [`for_each_part_mut`], but `f` returns a [`ControlFlow`] so the
    /// walk can stop early, [`ControlFlow::Break`] halts and returns its value,
    /// [`ControlFlow::Continue`] proceeds. Returns `ControlFlow::Continue(())` on
    /// a full walk, an unknown part, or a modality mismatch.
    ///
    /// [`for_each_part_mut`]: Self::for_each_part_mut
    // Returns a concrete `ControlFlow<B>` rather than being generic over
    // `Try`, which is nightly-only (`try_trait_v2`); revisit when it
    // stabilizes, see issue #139.
    pub fn try_for_each_part_mut<P: Modality, B>(
        &mut self,
        part_id: &PartId,
        f: impl FnMut(&mut Entity<P>) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        match self.part_entities_mut::<P>(part_id) {
            Some(entities) => entities.iter_mut().try_for_each(f),
            None => ControlFlow::Continue(()),
        }
    }

    /// The [`PartId`]s of the parts the report carries, paired with each part's
    /// modality [`TypeId`], for a caller enumerating what's editable. Includes
    /// every named document's own depth-1 part as well as any nested container
    /// parts.
    pub fn part_ids(&self) -> impl Iterator<Item = (&PartId, TypeId)> {
        self.parts.iter().map(|(id, p)| (id, p.modality))
    }

    /// The single top-level (depth-1) document's entry, its [`PartId`] and its
    /// part, if the report describes **exactly one** such document. The one
    /// place the "exactly one depth-1 part" rule lives, so the read
    /// ([`sole_document`](Self::sole_document)) and write
    /// ([`sole_document_id`](Self::sole_document_id)) shorthands cannot drift.
    /// `None` for zero or more than one top-level document (a multi-document set,
    /// addressed by name via [`part_entities`](Self::part_entities)).
    fn sole_document_entry(&self) -> Option<(&PartId, &PartReport)> {
        let mut tops = self.parts.iter().filter(|(id, _)| id.depth() == 1);
        tops.next().filter(|_| tops.next().is_none())
    }

    /// The single top-level document's part, if there is exactly one, the
    /// backing for the [`entities`](Self::entities) single-document shorthand.
    fn sole_document(&self) -> Option<&PartReport> {
        self.sole_document_entry().map(|(_, part)| part)
    }

    /// The [`PartId`] of the single top-level document, if there is exactly one,
    /// the `&mut` companion to [`sole_document`](Self::sole_document), which
    /// hands back an owned key so the borrow of `self.parts` is released before
    /// the caller re-borrows it mutably.
    fn sole_document_id(&self) -> Option<PartId> {
        self.sole_document_entry().map(|(id, _)| id.clone())
    }
}

/// Ensure `entity` carries a [`Manual`] event, recording one (from its own
/// location and confidence) if it does not already have one. Used by
/// [`Report::include_part`] so a manually-added entity is always auditable as a
/// human decision, however the caller built it.
///
/// [`Manual`]: elide_core::entity::audit::AuditKind::Manual
fn ensure_manual<M: Modality>(entity: &mut Entity<M>) {
    let has_manual = entity
        .audit
        .events()
        .iter()
        .any(|e| matches!(e.kind, AuditKind::Manual(_)));
    if !has_manual {
        let event = AuditEvent::manual_flag(entity.location.clone(), entity.confidence);
        entity.audit.record(event);
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::LabelRef;
    use elide_core::entity::audit::ManualIntent;
    use elide_core::modality::text::Text;

    use super::super::test_support::{doc, text_entity};
    use super::*;

    #[test]
    fn entity_mut_addresses_the_sole_document_by_id() {
        let a = text_entity("EMAIL_ADDRESS");
        let b = text_entity("PHONE_NUMBER");
        let (id_a, id_b) = (a.id, b.id);
        // A single depth-1 part, the sole document `entity`/`entity_mut` read.
        let mut report = Report::new().insert_part::<Text>(doc(), vec![a, b]);

        // A present id resolves to that exact entity.
        assert_eq!(
            report.entity_mut::<Text>(id_b).map(|e| e.label.as_str()),
            Some("PHONE_NUMBER")
        );
        assert_eq!(
            report.entity_mut::<Text>(id_a).map(|e| e.label.as_str()),
            Some("EMAIL_ADDRESS")
        );
        // An unknown id misses.
        assert!(report.entity_mut::<Text>(Uuid::now_v7()).is_none());
    }

    #[test]
    fn entities_is_sole_document_shorthand() {
        // Exactly one top-level document → `entities` reads it.
        let report = Report::new().insert_part::<Text>(doc(), vec![text_entity("EMAIL_ADDRESS")]);
        assert_eq!(report.entities::<Text>().map(<[_]>::len), Some(1));

        // Zero documents → None.
        assert!(Report::new().entities::<Text>().is_none());

        // More than one top-level document → None (a multi-document set; the
        // caller must address a part by name via `part_entities`).
        let two = Report::new()
            .insert_part::<Text>(PartId::new("a.txt"), vec![text_entity("EMAIL_ADDRESS")])
            .insert_part::<Text>(PartId::new("b.txt"), vec![text_entity("PHONE_NUMBER")]);
        assert!(
            two.entities::<Text>().is_none(),
            "a set has no sole document"
        );
        // But each is reachable by name.
        assert_eq!(
            two.part_entities::<Text>(&PartId::new("a.txt"))
                .map(<[_]>::len),
            Some(1)
        );

        // A nested container part does not count as a top-level document, so a
        // lone document with one nested part still has a sole document.
        let nested = Report::new()
            .insert_part::<Text>(doc(), vec![text_entity("EMAIL_ADDRESS")])
            .insert_part::<Text>(doc().child("word/media/image1.png"), vec![text_entity("X")]);
        assert_eq!(
            nested.entities::<Text>().map(<[_]>::len),
            Some(1),
            "only the depth-1 part is the sole document",
        );
    }

    #[test]
    fn include_part_on_the_sole_document_is_manual_and_readable_via_entities() {
        let mut report =
            Report::new().insert_part::<Text>(doc(), vec![text_entity("EMAIL_ADDRESS")]);
        let manual = text_entity("PHONE_NUMBER");
        let manual_id = manual.id;

        assert!(report.include_part::<Text>(&doc(), manual), "included");
        assert_eq!(report.entities::<Text>().unwrap().len(), 2);
        // The included entity, built here with only a Pattern event, now
        // carries a Manual event, so it is auditable as a human decision.
        let included = report.entity_mut::<Text>(manual_id).unwrap();
        assert!(
            included
                .audit
                .events()
                .iter()
                .any(|e| matches!(e.kind, AuditKind::Manual(_))),
            "include_part stamps a Manual event",
        );
        assert!(included.audit.verify().is_ok());

        // Including into an unknown part adds nothing and says so.
        assert!(!Report::new().include_part::<Text>(&doc(), text_entity("X")));
    }

    #[test]
    fn record_manual_amend_is_recorded_and_does_not_suppress() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id;
        let mut report = Report::new().insert_part::<Text>(doc(), vec![entity]);

        let e = report.entity_mut::<Text>(id).unwrap();
        assert!(e.record_manual(ManualIntent::Amend, None, Some("reviewer-7")));
        assert!(!e.is_suppressed(), "amend does not suppress");
        assert!(e.audit.verify().is_ok());
    }

    #[test]
    fn suppress_part_marks_a_part_entity_and_audits_it() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id;
        let part = PartId::new("word/media/image1.png");
        let mut report = Report::new().insert_part::<Text>(part.clone(), vec![entity]);

        assert!(report.suppress_part::<Text>(
            &part,
            id,
            Some(Attribution::freeform("false positive").into()),
            Some("reviewer-7".into()),
        ));

        let e = report.part_entity_mut::<Text>(&part, id).unwrap();
        assert!(e.is_suppressed(), "the part entity is marked suppressed");
        let has_manual = e.audit.events().iter().any(|ev| {
            matches!(&ev.kind, elide_core::entity::audit::AuditKind::Manual(m)
                if m.attribution == Some(Attribution::freeform("false positive").into()))
        });
        assert!(has_manual, "the suppression is audited");
        assert!(e.audit.verify().is_ok());

        // An unknown part, and an unknown id in a known part, both do nothing.
        assert!(!report.suppress_part::<Text>(&PartId::new("missing"), id, None, None::<String>,));
        assert!(!report.suppress_part::<Text>(&part, Uuid::now_v7(), None, None::<String>));
    }

    #[test]
    fn include_part_adds_a_manual_entity_to_a_part() {
        let part = PartId::new("word/media/image1.png");
        let mut report = Report::new().insert_part::<Text>(part.clone(), Vec::new());

        let included = text_entity("EMAIL_ADDRESS");
        let included_id = included.id;
        assert!(
            report.include_part::<Text>(&part, included),
            "included into the part",
        );
        let part_entities = report.part_entities::<Text>(&part).unwrap();
        assert_eq!(part_entities.len(), 1);
        // The included part entity is stamped with a Manual event.
        assert!(
            part_entities[0]
                .audit
                .events()
                .iter()
                .any(|e| matches!(e.kind, AuditKind::Manual(_))),
            "include_part stamps a Manual event",
        );
        assert_eq!(part_entities[0].id, included_id);

        // An unknown part includes nothing.
        assert!(!report.include_part::<Text>(&PartId::new("missing"), text_entity("X")));
    }

    #[test]
    fn include_custom_adds_a_manual_entity_in_one_call() {
        use elide_core::modality::text::{Text, TextLocation};

        // include_custom targets the sole document: one depth-1 part.
        let mut report = Report::new().insert_part::<Text>(doc(), Vec::new());
        assert!(
            report.include_custom::<Text>("US_SSN", TextLocation::new(0, 9)),
            "custom entity added to the sole document",
        );
        let entities = report.entities::<Text>().expect("sole document entities");
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].label.as_str(), "US_SSN");
        // Built as a custom entity: MAX confidence + a Manual event.
        assert_eq!(
            entities[0].confidence,
            elide_core::primitive::Confidence::MAX
        );
        assert!(
            entities[0]
                .audit
                .events()
                .iter()
                .any(|e| matches!(e.kind, AuditKind::Manual(_))),
        );

        // A multi-document report has no *sole* document, so include_custom
        // returns false rather than guessing which document to add to.
        let mut multi = Report::new()
            .insert_part::<Text>(PartId::new("a.txt"), Vec::new())
            .insert_part::<Text>(PartId::new("b.txt"), Vec::new());
        assert!(
            !multi.include_custom::<Text>("US_SSN", TextLocation::new(0, 9)),
            "no sole document to add to",
        );
        // But include_custom_at addresses a specific part.
        assert!(multi.include_custom_at::<Text>(
            &PartId::new("a.txt"),
            "US_SSN",
            TextLocation::new(0, 9),
        ),);
        assert_eq!(
            multi
                .part_entities::<Text>(&PartId::new("a.txt"))
                .unwrap()
                .len(),
            1,
        );
        // An unknown part adds nothing.
        assert!(!multi.include_custom_at::<Text>(
            &PartId::new("missing"),
            "US_SSN",
            TextLocation::new(0, 9),
        ));
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
        assert!(
            report
                .part_entity_mut::<Text>(&part, Uuid::now_v7())
                .is_none()
        );
    }

    #[test]
    fn entity_anywhere_reaches_a_nested_part_by_id_alone() {
        // A review layer holds only an entity id (a retag/suppress addresses the
        // entity, not the part it lives in). The entity lives in a *nested*
        // depth-2 part, so the report is not a single sole document.
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id;
        let nested = PartId::new("report.docx").child("word/media/image1.png");
        let mut report = Report::new().insert_part::<Text>(nested.clone(), vec![entity]);

        // The sole-document sugar cannot see it (the report is not a single
        // depth-1 document), so a caller relying on `entity_mut` would silently
        // find nothing — exactly why `entity_anywhere` exists.
        assert!(
            report.entity_mut::<Text>(id).is_none(),
            "entity_mut is sole-document sugar and must not resolve a nested part",
        );

        // `entity_anywhere` finds it by id alone, and `_mut` edits it in place.
        assert_eq!(
            report.entity_anywhere::<Text>(id).map(|e| e.label.as_str()),
            Some("EMAIL_ADDRESS"),
        );
        report
            .entity_anywhere_mut::<Text>(id)
            .expect("nested entity is reachable by id alone")
            .label = LabelRef::new("RETAGGED");
        assert_eq!(
            report.part_entity::<Text>(&nested, id).unwrap().label,
            LabelRef::new("RETAGGED"),
            "the edit applied to the nested part",
        );

        // An unknown id resolves nowhere.
        assert!(report.entity_anywhere::<Text>(Uuid::now_v7()).is_none());
    }

    #[test]
    fn for_each_part_mut_visits_every_entity_once() {
        let part = doc();
        let mut report = Report::new().insert_part::<Text>(
            part.clone(),
            vec![text_entity("EMAIL_ADDRESS"), text_entity("PHONE_NUMBER")],
        );

        // The pass mutates in place and visits each entity exactly once.
        let mut count = 0;
        report.for_each_part_mut::<Text>(&part, |e| {
            count += 1;
            e.label = LabelRef::new("REDACTED");
        });
        assert_eq!(count, 2);
        assert!(
            report
                .part_entities::<Text>(&part)
                .unwrap()
                .iter()
                .all(|e| e.label.as_str() == "REDACTED")
        );

        // An unknown part → the closure never runs.
        let mut ran = false;
        report.for_each_part_mut::<Text>(&PartId::new("nope"), |_| ran = true);
        assert!(!ran);
    }

    #[test]
    fn try_for_each_part_mut_breaks_early() {
        let a = text_entity("EMAIL_ADDRESS");
        let target = a.id;
        let part = doc();
        let mut report =
            Report::new().insert_part::<Text>(part.clone(), vec![a, text_entity("PHONE_NUMBER")]);

        // Break carries a value out and halts the walk at the first match.
        let mut visited = 0;
        let found = report.try_for_each_part_mut::<Text, &'static str>(&part, |e| {
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
        let done = report.try_for_each_part_mut::<Text, ()>(&part, |_| ControlFlow::Continue(()));
        assert_eq!(done, ControlFlow::Continue(()));

        // An unknown part → vacuously Continue.
        let empty = report
            .try_for_each_part_mut::<Text, ()>(&PartId::new("nope"), |_| ControlFlow::Break(()));
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
