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

use elide_codec::{PartId, UntypedDocumentHandle};
use elide_core::entity::Entity;
use elide_core::entity::audit::{Attribution, AuditEvent, AuditKind, ManualIntent};
use elide_core::modality::Modality;
#[cfg(feature = "usage")]
use elide_core::recognition::UsageReport;
use uuid::Uuid;

use super::group::EntityGroup;
use super::registry::ReportDeserializer;

/// The document body's entry: its detected entities and the modality they
/// belong to.
///
/// A document has exactly one body, so a [`Report`] holds at most one of
/// these. The `modality` is the routing key — the pipeline registered for it
/// analyzes and applies the body.
///
/// The body's *enrichment* (OCR `Layout`, STT `Transcription`) is not held here:
/// a [`Report`] is references only, like its entities. Enrichment content lives
/// in the parallel [`ArtifactSet`](crate::ArtifactSet).
pub(crate) struct BodyReport {
    /// The body's modality, keying the pipeline that handles it.
    pub(crate) modality: TypeId,
    /// The body's detected entities (a `Vec<Entity<M>>`).
    pub(crate) entities: Box<dyn EntityGroup>,
}

/// One container part captured during analysis: its detected entities, the
/// modality they belong to, and — for the same-process fast path — the
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
    /// Never serialized — a live decoded document is not data.
    ///
    /// [`analyze`]: crate::Orchestrator::analyze
    pub(crate) handle: Option<UntypedDocumentHandle>,
    /// The part's detected entities (a `Vec<Entity<P>>`).
    pub(crate) entities: Box<dyn EntityGroup>,
}

/// The detected entities of a whole document, editable before apply.
///
/// Returned by [`analyze`] and consumed by [`anonymize_with`]. Read the body
/// entities of modality `M` with [`entities`] (a part's with [`part_entities`]),
/// returning a `&[Entity<_>]`; edit them with [`entities_mut`] /
/// [`part_entities_mut`], returning a `&mut Vec<Entity<_>>` you can filter,
/// retag, or extend before applying. To reach one entity by [`id`] use
/// [`entity`] / [`entity_mut`] (body) or [`part_entity`] / [`part_entity_mut`]
/// (a named part); a review layer that holds an id without knowing where it
/// lives uses [`entity_anywhere`] / [`entity_anywhere_mut`], which sweep the
/// body and every part. To walk a whole group in a single mutable pass (e.g.
/// merging the applied report's provenance back onto a caller's records) use
/// [`for_each_body_mut`] / [`for_each_part_mut`], or their `try_` variants to
/// stop the walk early.
///
/// [`id`]: elide_core::entity::Entity::id
/// [`entity`]: Report::entity
/// [`entity_mut`]: Report::entity_mut
/// [`part_entity`]: Report::part_entity
/// [`part_entity_mut`]: Report::part_entity_mut
/// [`entity_anywhere`]: Report::entity_anywhere
/// [`entity_anywhere_mut`]: Report::entity_anywhere_mut
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
/// [`analyze`]: crate::Orchestrator::analyze
/// [`anonymize_with`]: crate::Orchestrator::anonymize_with
/// [`entities`]: Report::entities
/// [`entities_mut`]: Report::entities_mut
/// [`part_entities`]: Report::part_entities
/// [`part_entities_mut`]: Report::part_entities_mut
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
    /// Per-recognizer / per-enricher resource usage across the whole
    /// analysis (the body and every part), in run order.
    #[cfg(feature = "usage")]
    pub(crate) usage: UsageReport,
}

impl Report {
    /// An empty report — no body, no parts. Fill it with [`insert_body`]
    /// and [`insert_part`], or let [`analyze`] produce one.
    ///
    /// [`insert_body`]: Self::insert_body
    /// [`insert_part`]: Self::insert_part
    /// [`analyze`]: crate::Orchestrator::analyze
    pub fn new() -> Self {
        Self {
            body: None,
            parts: HashMap::new(),
            #[cfg(feature = "usage")]
            usage: UsageReport::new(),
        }
    }

    /// A [`ReportDeserializer`] for reconstructing a serialized report — without
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

    /// The resource usage recorded across this analysis — one entry per
    /// recognizer and enricher that ran, each self-identifying via its id.
    #[cfg(feature = "usage")]
    #[must_use]
    pub fn usage(&self) -> &UsageReport {
        &self.usage
    }

    /// Set the body entities of modality `M`, replacing any already set.
    ///
    /// For rebuilding a report from out-of-band entities (e.g. deserialized
    /// from a review tool). [`anonymize_with`] reads these back through the
    /// `M` pipeline.
    ///
    /// [`anonymize_with`]: crate::Orchestrator::anonymize_with
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
    /// [`anonymize_with`]: crate::Orchestrator::anonymize_with
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

    /// The body entities of modality `M`, read-only. Returns `None` if the
    /// document's body is a different modality (or no body pipeline ran). Use
    /// [`entities_mut`] to edit them.
    ///
    /// [`entities_mut`]: Self::entities_mut
    pub fn entities<M: Modality>(&self) -> Option<&[Entity<M>]> {
        let body = self.body.as_ref()?;
        if body.modality != TypeId::of::<M>() {
            return None;
        }
        body.entities
            .as_any()
            .downcast_ref::<Vec<Entity<M>>>()
            .map(Vec::as_slice)
    }

    /// The body entities of modality `M`, for editing — the `&mut` counterpart
    /// to [`entities`]. Returns `None` if the body is a different modality, or
    /// no body pipeline ran.
    ///
    /// [`entities`]: Self::entities
    pub fn entities_mut<M: Modality>(&mut self) -> Option<&mut Vec<Entity<M>>> {
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
        self.entities_mut::<M>()?.iter_mut().find(|e| e.id == id)
    }

    /// One body entity of modality `M` by its [`id`], read-only — the `&`
    /// counterpart to [`entity_mut`]. Returns `None` when the body is a
    /// different modality than `M` or no entity has that `id`.
    ///
    /// [`id`]: elide_core::entity::Entity::id
    /// [`entity_mut`]: Self::entity_mut
    pub fn entity<M: Modality>(&self, id: Uuid) -> Option<&Entity<M>> {
        self.entities::<M>()?.iter().find(|e| e.id == id)
    }

    /// Find an entity of modality `M` by its [`id`] **anywhere** in the report —
    /// the body, then every container part — read-only. For a review layer that
    /// holds an entity id with no indication of where it lives.
    ///
    /// Parts of a different modality than `M` are skipped. Returns `None` when
    /// no entity of modality `M` with that `id` exists in the body or any part.
    /// Use [`entity_anywhere_mut`] to edit the match.
    ///
    /// [`id`]: elide_core::entity::Entity::id
    /// [`entity_anywhere_mut`]: Self::entity_anywhere_mut
    pub fn entity_anywhere<M: Modality>(&self, id: Uuid) -> Option<&Entity<M>> {
        if let Some(entity) = self.entity::<M>(id) {
            return Some(entity);
        }
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

    /// Find an entity of modality `M` by its [`id`] **anywhere** in the report —
    /// the body, then every container part — for editing. The `&mut`
    /// counterpart to [`entity_anywhere`], for a review layer acting on an id
    /// whose location it does not track.
    ///
    /// Parts of a different modality than `M` are skipped. Returns `None` when
    /// no entity of modality `M` with that `id` exists in the body or any part.
    ///
    /// [`id`]: elide_core::entity::Entity::id
    /// [`entity_anywhere`]: Self::entity_anywhere
    pub fn entity_anywhere_mut<M: Modality>(&mut self, id: Uuid) -> Option<&mut Entity<M>> {
        // Split the borrow: check the body first, then fall through to the
        // parts. `entity_mut` would hold `&mut self` across the parts search, so
        // the two lookups are inlined here to keep the borrows disjoint.
        if self.entity::<M>(id).is_some() {
            return self.entity_mut::<M>(id);
        }
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

    /// Manually add `entity` to the body group — a reviewer including a
    /// detection the engine missed. Its human origin is made auditable: if
    /// `entity` does not already carry a [`Manual`] event, one is recorded onto
    /// its trail (from its own location and confidence) as it is included, so an
    /// included entity is never mistaken for an automatic detection. Returns
    /// `false` (adding nothing) when no body pipeline ran or the body is a
    /// different modality than `M` — use [`insert_body`](Self::insert_body) to
    /// seed an empty report first.
    ///
    /// [`Manual`]: elide_core::entity::audit::AuditKind::Manual
    pub fn include<M: Modality>(&mut self, mut entity: Entity<M>) -> bool {
        ensure_manual(&mut entity);
        match self.entities_mut::<M>() {
            Some(entities) => {
                entities.push(entity);
                true
            }
            None => false,
        }
    }

    /// Manually suppress the body entity `id` — a reviewer marking a detection
    /// to leave alone (a false positive). It stays in the report but the
    /// redaction pass skips it; a [`Manual`] event built from the entity's own
    /// location and confidence, carrying the `attribution` (*why*) and `actor`
    /// (*who*, recorded as the event's source), is recorded so it is auditable.
    /// Idempotent: suppressing an already-suppressed entity records nothing (see
    /// [`Entity::record_manual`]). Returns `false` only when the body is a
    /// different modality than `M` or no entity has that `id`.
    ///
    /// [`Manual`]: elide_core::entity::audit::AuditKind::Manual
    /// [`Entity::record_manual`]: elide_core::entity::Entity::record_manual
    pub fn suppress<M: Modality>(
        &mut self,
        id: Uuid,
        attribution: Option<Attribution>,
        actor: Option<String>,
    ) -> bool {
        match self.entity_mut::<M>(id) {
            Some(entity) => {
                entity.record_manual(ManualIntent::Suppress, attribution, actor.as_deref());
                true
            }
            None => false,
        }
    }

    /// Manually add `entity` to the container part `part_id` — the part
    /// counterpart to [`include`], recording a [`Manual`] event onto `entity`
    /// (unless it already carries one) so its human origin is auditable. Returns
    /// `false` for an unknown part or a modality mismatch.
    ///
    /// [`include`]: Self::include
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

    /// Manually suppress the entity `id` in the container part `part_id` — the
    /// part counterpart to [`suppress`], so a reviewer can leave alone a false
    /// positive detected inside a part. Records the same auditable [`Manual`]
    /// event. Returns `false` for an unknown part, a modality mismatch, or no
    /// entity with that `id`.
    ///
    /// [`suppress`]: Self::suppress
    /// [`Manual`]: elide_core::entity::audit::AuditKind::Manual
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
        if let Some(entities) = self.entities_mut::<M>() {
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
        match self.entities_mut::<M>() {
            Some(entities) => entities.iter_mut().try_for_each(f),
            None => ControlFlow::Continue(()),
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

    /// The entities of the container part `id`, as modality `P`, for editing —
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
    /// [`id`], read-only — the `&` counterpart to [`part_entity_mut`]. Returns
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
        if let Some(entities) = self.part_entities_mut::<P>(part_id) {
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
        match self.part_entities_mut::<P>(part_id) {
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

/// Ensure `entity` carries a [`Manual`] event, recording one (from its own
/// location and confidence) if it does not already have one. Shared by
/// [`Report::include`] and [`Report::include_part`] so a manually-added entity
/// is always auditable as a human decision, however the caller built it.
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
    use elide_core::entity::audit::{AuditEvent, AuditLog, ManualIntent, PatternEvent};
    use elide_core::entity::{Entity, LabelRef};
    use elide_core::modality::text::{Text, TextLocation};
    use elide_core::primitive::Confidence;

    use super::*;

    /// A minimal text entity carrying `label`, for building reports under test.
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
    fn include_adds_a_manual_entity_to_the_body() {
        let mut report = Report::new().insert_body::<Text>(vec![text_entity("EMAIL_ADDRESS")]);
        let manual = text_entity("PHONE_NUMBER");
        let manual_id = manual.id;

        assert!(report.include::<Text>(manual), "included into the body");
        assert_eq!(report.entities::<Text>().unwrap().len(), 2);
        // The included entity — built here with only a Pattern event — now
        // carries a Manual event stamped by `include`, so it is auditable as a
        // human decision.
        let included = report.entity_mut::<Text>(manual_id).unwrap();
        assert!(
            included
                .audit
                .events()
                .iter()
                .any(|e| matches!(e.kind, AuditKind::Manual(_))),
            "include stamps a Manual event",
        );
        assert!(included.audit.verify().is_ok());

        // Including on an empty report (no body) adds nothing and says so.
        assert!(!Report::new().include::<Text>(text_entity("X")));
    }

    #[test]
    fn suppress_marks_the_entity_and_audits_it() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id;
        let mut report = Report::new().insert_body::<Text>(vec![entity]);

        assert!(report.suppress::<Text>(
            id,
            Some(Attribution::freeform("false positive").into()),
            Some("reviewer-7".into())
        ));

        let e = report.entity_mut::<Text>(id).unwrap();
        assert!(e.is_suppressed(), "the entity is marked suppressed");
        // The suppression is on the audit trail: the *why* is the Manual event's
        // attribution, the *who* is its source.
        let manual = e
            .audit
            .events()
            .iter()
            .find_map(|ev| match &ev.kind {
                elide_core::entity::audit::AuditKind::Manual(m) => {
                    Some((m.attribution.clone(), ev.source.clone()))
                }
                _ => None,
            })
            .expect("a Manual event was recorded");
        assert_eq!(
            manual.0,
            Some(Attribution::freeform("false positive").into())
        );
        assert_eq!(manual.1, "reviewer-7");
        // The audit chain still verifies with the appended Manual event.
        assert!(e.audit.verify().is_ok());

        // Idempotent: suppressing again returns true (it is suppressed) but
        // records no duplicate Manual event.
        let before = report.entity::<Text>(id).unwrap().audit.events().len();
        assert!(report.suppress::<Text>(id, None, None::<String>));
        let after = report.entity::<Text>(id).unwrap().audit.events().len();
        assert_eq!(before, after, "re-suppressing does not grow the trail");

        // Suppressing an unknown id does nothing.
        assert!(!report.suppress::<Text>(Uuid::now_v7(), None, None::<String>));
    }

    #[test]
    fn entity_anywhere_finds_body_and_part_entities() {
        let body = text_entity("EMAIL_ADDRESS");
        let body_id = body.id;
        let part_entity = text_entity("PHONE_NUMBER");
        let part_id = part_entity.id;
        let part = PartId::new("word/document2.xml");
        let mut report = Report::new()
            .insert_body::<Text>(vec![body])
            .insert_part::<Text>(part, vec![part_entity]);

        // Both are found by id alone, without the caller knowing where each lives.
        assert_eq!(
            report.entity_anywhere::<Text>(body_id).map(|e| e.id),
            Some(body_id)
        );
        assert_eq!(
            report.entity_anywhere::<Text>(part_id).map(|e| e.id),
            Some(part_id)
        );
        // The mut sweep reaches a part entity for editing.
        assert!(report.entity_anywhere_mut::<Text>(part_id).is_some());
        // An unknown id resolves nowhere.
        assert!(report.entity_anywhere::<Text>(Uuid::now_v7()).is_none());
    }

    #[test]
    fn record_manual_amend_is_recorded_and_does_not_suppress() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id;
        let mut report = Report::new().insert_body::<Text>(vec![entity]);

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
