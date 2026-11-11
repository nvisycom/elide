//! [`Report`]: the detected-but-not-yet-applied entities of a document set,
//! editable per part before [`anonymize_with`].
//!
//! Detection (`analyze`) and redaction (`anonymize_with`) are split so a
//! reviewer can inspect and edit the entities in between: suppress a false
//! positive, include a missed one, retag or retarget a span, each recorded on
//! the entity's audit trail. A document set's entities span
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
use std::collections::{BTreeMap, HashMap};

use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::redaction::HiddenContent;
use elide_core::{Error, Result};
use uuid::Uuid;

use super::group::EntityGroup;
use super::part::{Part, PartMut};
use super::registry::ReportDeserializer;
use crate::PartId;

/// One part captured during analysis, a named document's own content (a
/// depth-1 part) or a container part nested within one: its detected entities
/// and the modality they belong to.
pub(crate) struct PartReport {
    /// The part's modality, the routing key for [`anonymize_with`]: it re-walks
    /// the document's part tree to this part and applies through the pipeline
    /// registered for this modality.
    ///
    /// [`anonymize_with`]: crate::Orchestrator::anonymize_with
    pub(crate) modality: TypeId,
    /// The part's detected entities (a `Vec<Entity<P>>`).
    pub(crate) entities: Box<dyn EntityGroup>,
}

/// The detected entities of a document set, editable before apply.
///
/// Returned by [`analyze`] and consumed by [`anonymize_with`]. Every part,
/// each named document's own content, and every container part within one, is
/// keyed by its [`PartId`] path. Read a part as modality `M` with [`part`], or
/// edit it with [`part_mut`]: the views resolve the part and check its modality
/// once. [`document`] and [`document_mut`] reach the report's single top-level
/// document. A reviewer's edits through a [`PartMut`] (including a missed
/// entity, suppressing a false positive, suppressing everything a region
/// contains) are recorded on the entities' own audit trails, so the report
/// carries them to [`anonymize_with`] and to whoever audits it after. To reach
/// an entity when only its [`id`] is known, use [`entity_anywhere`] /
/// [`entity_anywhere_mut`].
///
/// A report is **pure entity data**: it carries no live document state, so
/// it can be built from scratch ([`new`] + [`insert_part`]) and, with the
/// `serde` feature, serialized to a part-list `{ parts: [..] }` view, shipped
/// elsewhere, and reconstructed there. [`anonymize_with`] re-decodes each part
/// from the container it is applied to, so a rebuilt report redacts just as a
/// freshly-analyzed one does.
///
/// [`analyze`]: crate::Orchestrator::analyze
/// [`anonymize_with`]: crate::Orchestrator::anonymize_with
/// [`part`]: Report::part
/// [`part_mut`]: Report::part_mut
/// [`document`]: Report::document
/// [`document_mut`]: Report::document_mut
/// [`id`]: elide_core::entity::Entity::id
/// [`entity_anywhere`]: Report::entity_anywhere
/// [`entity_anywhere_mut`]: Report::entity_anywhere_mut
/// [`new`]: Report::new
/// [`insert_part`]: Report::insert_part
#[derive(Default)]
pub struct Report {
    /// Every part's entry, keyed by its [`PartId`] path, a named document's own
    /// content is its depth-1 part, container parts nest below.
    pub(crate) parts: HashMap<PartId, PartReport>,
    /// The hidden content the last walk over the documents saw stripped.
    pub(crate) stripped: StrippedRecord,
}

/// The hidden content a walk saw stripped, keyed by part and item, with how
/// many. Each walk builds its own and hands it to the report whole.
#[derive(Debug, Default)]
pub(crate) struct StrippedRecord(BTreeMap<(PartId, HiddenContent), usize>);

impl StrippedRecord {
    /// Record `count` of `content` stripped from `part`.
    pub(crate) fn record(&mut self, part: PartId, content: HiddenContent, count: usize) {
        if count > 0 {
            *self.0.entry((part, content)).or_default() += count;
        }
    }
}

/// One item of hidden content stripped from a document, as a [`Report`]
/// records it: where, what, and how many. Never the stripped value itself, so
/// the report can be shared without shipping what was removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrippedItem {
    /// The part it was stripped from: a document's own path, or a part within
    /// it (a removed or edited package part, an embedded file).
    pub part: PartId,
    /// The item stripped.
    pub content: HiddenContent,
    /// How many were stripped: elements, parts, blocks, or objects, as the
    /// format counts them.
    pub count: usize,
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
            stripped: StrippedRecord::default(),
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

    /// Every item of hidden content stripped from the documents, ordered by part
    /// then item.
    ///
    /// The section is output only: a record of what a walk over the documents
    /// saw, never a decision a reviewer edits, so it is serialized but never
    /// read back. [`analyze`] fills it, so the removals can be reviewed before
    /// anything is redacted, and [`anonymize_with`] replaces it with what its
    /// own walk saw, so the applied report is the audit of that run whatever
    /// report was passed in.
    ///
    /// [`analyze`]: crate::Orchestrator::analyze
    /// [`anonymize_with`]: crate::Orchestrator::anonymize_with
    pub fn stripped(&self) -> impl Iterator<Item = StrippedItem> + '_ {
        self.stripped
            .0
            .iter()
            .map(|((part, content), &count)| StrippedItem {
                part: part.clone(),
                content: *content,
                count,
            })
    }

    /// The items of hidden content stripped from the part `id` itself.
    pub fn part_stripped<'a>(&'a self, id: &'a PartId) -> impl Iterator<Item = StrippedItem> + 'a {
        self.stripped().filter(move |item| &item.part == id)
    }

    /// Replace the stripped section with the `record` a walk built.
    pub(crate) fn set_stripped(&mut self, record: StrippedRecord) {
        self.stripped = record;
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
                entities: Box::new(entities),
            },
        );
        self
    }

    /// The part `id`, as modality `M`, to read.
    ///
    /// # Errors
    ///
    /// [`NotFound`](elide_core::ErrorKind::NotFound) when the report holds no
    /// part `id`, or holds it as another modality.
    pub fn part<M: Modality>(&self, id: &PartId) -> Result<Part<'_, M>> {
        let part = self
            .parts
            .get(id)
            .ok_or_else(|| Error::not_found(format!("the report holds no part `{id}`")))?;
        Ok(Part::new(typed::<M>(id, part)?))
    }

    /// The part `id`, as modality `M`, to edit.
    ///
    /// # Errors
    ///
    /// [`NotFound`](elide_core::ErrorKind::NotFound) when the report holds no
    /// part `id`, or holds it as another modality.
    pub fn part_mut<M: Modality>(&mut self, id: &PartId) -> Result<PartMut<'_, M>> {
        let part = self
            .parts
            .get_mut(id)
            .ok_or_else(|| Error::not_found(format!("the report holds no part `{id}`")))?;
        Ok(PartMut::new(typed_mut::<M>(id, part)?))
    }

    /// The report's single top-level document, as modality `M`, to read.
    ///
    /// # Errors
    ///
    /// [`NotFound`](elide_core::ErrorKind::NotFound) when the report holds no
    /// top-level document or more than one, or holds it as another modality.
    pub fn document<M: Modality>(&self) -> Result<Part<'_, M>> {
        let id = self.sole_document_id()?;
        self.part::<M>(&id)
    }

    /// The report's single top-level document, as modality `M`, to edit.
    ///
    /// # Errors
    ///
    /// [`NotFound`](elide_core::ErrorKind::NotFound) when the report holds no
    /// top-level document or more than one, or holds it as another modality.
    pub fn document_mut<M: Modality>(&mut self) -> Result<PartMut<'_, M>> {
        let id = self.sole_document_id()?;
        self.part_mut::<M>(&id)
    }

    /// Find an entity of modality `M` by its [`id`] across **every** part,
    /// read-only, for a caller that holds an entity's id but not the part it
    /// lives in. Parts of another modality are skipped.
    ///
    /// [`id`]: elide_core::entity::Entity::id
    pub fn entity_anywhere<M: Modality>(&self, id: Uuid) -> Option<&Entity<M>> {
        self.parts
            .iter()
            .filter_map(|(part_id, part)| typed::<M>(part_id, part).ok())
            .find_map(|entities| entities.iter().find(|entity| entity.id == id))
    }

    /// Find an entity of modality `M` by its [`id`] across **every** part, for
    /// editing, the `&mut` counterpart to [`entity_anywhere`]. Parts of another
    /// modality are skipped.
    ///
    /// [`id`]: elide_core::entity::Entity::id
    /// [`entity_anywhere`]: Self::entity_anywhere
    pub fn entity_anywhere_mut<M: Modality>(&mut self, id: Uuid) -> Option<&mut Entity<M>> {
        self.parts
            .iter_mut()
            .filter_map(|(part_id, part)| typed_mut::<M>(part_id, part).ok())
            .find_map(|entities| entities.iter_mut().find(|entity| entity.id == id))
    }

    /// The [`PartId`]s of the parts the report carries, paired with each part's
    /// modality [`TypeId`], for a caller enumerating what's editable. Includes
    /// every named document's own depth-1 part as well as any nested container
    /// parts.
    pub fn part_ids(&self) -> impl Iterator<Item = (&PartId, TypeId)> {
        self.parts.iter().map(|(id, p)| (id, p.modality))
    }

    /// The [`PartId`] of the single top-level (depth-1) document.
    fn sole_document_id(&self) -> Result<PartId> {
        let mut tops = self.parts.keys().filter(|id| id.depth() == 1);
        match (tops.next(), tops.next()) {
            (Some(id), None) => Ok(id.clone()),
            (None, _) => Err(Error::not_found("the report holds no document")),
            (Some(_), Some(_)) => Err(Error::not_found(
                "the report holds more than one document; address one with `part`",
            )),
        }
    }
}

/// The entities of `part` (keyed `id`), as modality `M`.
fn typed<'p, M: Modality>(id: &PartId, part: &'p PartReport) -> Result<&'p [Entity<M>]> {
    if part.modality != TypeId::of::<M>() {
        return Err(mismatch::<M>(id));
    }
    part.entities
        .as_any()
        .downcast_ref::<Vec<Entity<M>>>()
        .map(Vec::as_slice)
        .ok_or_else(|| mismatch::<M>(id))
}

/// The entities of `part` (keyed `id`), as modality `M`, to edit.
fn typed_mut<'p, M: Modality>(
    id: &PartId,
    part: &'p mut PartReport,
) -> Result<&'p mut Vec<Entity<M>>> {
    if part.modality != TypeId::of::<M>() {
        return Err(mismatch::<M>(id));
    }
    part.entities
        .as_any_mut()
        .downcast_mut::<Vec<Entity<M>>>()
        .ok_or_else(|| mismatch::<M>(id))
}

/// The error for a part `id` that is not of modality `M`.
fn mismatch<M: Modality>(id: &PartId) -> Error {
    Error::not_found(format!("part `{id}` is not {}", M::NAME))
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;
    use elide_core::entity::LabelRef;
    use elide_core::entity::audit::{Attribution, AuditKind, ManualIntent};
    use elide_core::modality::text::{Text, TextLocation};
    use elide_core::primitive::Confidence;

    use super::super::test_support::{doc, text_entity};
    use super::*;

    /// A [`Text`] entity labelled `label` at `range`.
    fn at(label: &str, range: std::ops::Range<usize>) -> Entity<Text> {
        let mut entity = text_entity(label);
        entity.location = TextLocation::inline(range);
        entity
    }

    #[test]
    fn a_part_is_read_and_edited_by_its_id() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id;
        let part = doc().child("word/media/image1.png");
        let mut report = Report::new().insert_part::<Text>(part.clone(), vec![entity]);

        assert_eq!(report.part::<Text>(&part).unwrap().entities().len(), 1);
        assert!(
            report
                .part_mut::<Text>(&part)
                .unwrap()
                .relabel(id, "RETAGGED")
        );
        assert_eq!(
            report
                .part::<Text>(&part)
                .unwrap()
                .entity(id)
                .unwrap()
                .label,
            LabelRef::new("RETAGGED"),
        );
    }

    #[test]
    fn an_unknown_part_or_another_modality_is_not_found() {
        let mut report = Report::new().insert_part::<Text>(doc(), vec![text_entity("X")]);
        let missing = report.part::<Text>(&PartId::new("missing")).unwrap_err();
        assert_eq!(missing.kind(), ErrorKind::NotFound);

        #[derive(Debug)]
        struct Other;
        impl Modality for Other {
            type Artifact = elide_core::modality::NoArtifact;
            type Data = <Text as Modality>::Data;
            type Location = <Text as Modality>::Location;
            type Replacement = <Text as Modality>::Replacement;

            const NAME: &'static str = "other";
        }
        assert_eq!(
            report.part_mut::<Other>(&doc()).unwrap_err().kind(),
            ErrorKind::NotFound,
        );
    }

    #[test]
    fn the_document_is_the_sole_top_level_part() {
        // One top-level document with a nested part: the document is its own.
        let report = Report::new()
            .insert_part::<Text>(doc(), vec![text_entity("EMAIL_ADDRESS")])
            .insert_part::<Text>(doc().child("word/media/image1.png"), vec![text_entity("X")]);
        assert_eq!(report.document::<Text>().unwrap().entities().len(), 1);

        // None, or more than one, is not found.
        assert_eq!(
            Report::new().document::<Text>().unwrap_err().kind(),
            ErrorKind::NotFound
        );
        let two = Report::new()
            .insert_part::<Text>(PartId::new("a.txt"), vec![text_entity("A")])
            .insert_part::<Text>(PartId::new("b.txt"), vec![text_entity("B")]);
        assert_eq!(
            two.document::<Text>().unwrap_err().kind(),
            ErrorKind::NotFound
        );
        assert_eq!(
            two.part::<Text>(&PartId::new("a.txt"))
                .unwrap()
                .entities()
                .len(),
            1
        );
    }

    #[test]
    fn an_included_entity_is_a_reviewers_assertion() {
        let mut report = Report::new().insert_part::<Text>(doc(), Vec::new());
        let id = report
            .document_mut::<Text>()
            .unwrap()
            .by("reviewer-7")
            .because(Attribution::freeform("missed by detection"))
            .include("PERSON_NAME", TextLocation::inline(4..12));

        let part = report.document::<Text>().unwrap();
        let entity = part.entity(id).expect("included");
        assert_eq!(entity.confidence, Confidence::MAX);
        let [event] = entity.audit.events() else {
            panic!("one event");
        };
        assert_eq!(event.source.as_str(), "reviewer-7");
        let AuditKind::Manual(manual) = &event.kind else {
            panic!("a Manual event");
        };
        assert_eq!(manual.intent, ManualIntent::Flag);
        assert_eq!(
            manual.attribution,
            Some(Attribution::freeform("missed by detection").into())
        );
        assert!(entity.audit.verify().is_ok());
        assert_eq!(part.unsuppressed().count(), 1);
    }

    #[test]
    fn a_suppressed_entity_stays_in_the_part_and_is_audited() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id;
        let mut report = Report::new().insert_part::<Text>(doc(), vec![entity]);

        let mut part = report.document_mut::<Text>().unwrap().by("reviewer-7");
        assert!(part.suppress(id), "suppressed");
        assert!(!part.suppress(id), "suppressing again records nothing");
        assert!(
            !part.suppress(Uuid::now_v7()),
            "an unknown id is not suppressed"
        );

        let part = report.document::<Text>().unwrap();
        assert_eq!(part.suppressed().count(), 1);
        assert_eq!(part.unsuppressed().count(), 0);
        let entity = part.entity(id).unwrap();
        assert!(entity.audit.events().iter().any(|event| {
            event.source.as_str() == "reviewer-7"
                && matches!(&event.kind, AuditKind::Manual(m) if m.intent == ManualIntent::Suppress)
        }));
        assert!(entity.audit.verify().is_ok());
    }

    #[test]
    fn a_retag_or_retarget_is_an_audited_amendment() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id;
        let mut report = Report::new().insert_part::<Text>(doc(), vec![entity]);

        let mut part = report
            .document_mut::<Text>()
            .unwrap()
            .by("reviewer-7")
            .because(Attribution::freeform("misread"));
        assert!(part.relabel(id, "PHONE_NUMBER"));
        assert!(part.relocate(id, TextLocation::inline(1..3)));
        assert!(
            !part.relabel(Uuid::now_v7(), "X"),
            "an unknown id is not amended"
        );

        let part = report.document::<Text>().unwrap();
        let entity = part.entity(id).unwrap();
        assert_eq!(entity.label, LabelRef::new("PHONE_NUMBER"));
        assert_eq!(entity.location, TextLocation::inline(1..3));
        let amendments: Vec<_> = entity
            .audit
            .events()
            .iter()
            .filter_map(|event| match &event.kind {
                AuditKind::Manual(m) if m.intent == ManualIntent::Amend => {
                    Some((event.source.as_str(), m.location.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            amendments,
            [
                ("reviewer-7", TextLocation::inline(0..4)),
                ("reviewer-7", TextLocation::inline(1..3)),
            ]
        );
        assert!(!entity.is_suppressed(), "an amendment does not suppress");
        assert!(entity.audit.verify().is_ok());
    }

    #[test]
    fn a_region_suppresses_only_what_it_contains() {
        let inside = at("EMAIL_ADDRESS", 4..10);
        let crossing = at("PERSON_NAME", 15..25);
        let outside = at("PHONE_NUMBER", 30..40);
        let (inside_id, crossing_id, outside_id) = (inside.id, crossing.id, outside.id);
        let mut report = Report::new().insert_part::<Text>(doc(), vec![inside, crossing, outside]);

        let region = TextLocation::inline(0..20);
        let spared = report
            .document_mut::<Text>()
            .unwrap()
            .suppress_within(&region);
        assert_eq!(spared, [inside_id]);

        let part = report.document::<Text>().unwrap();
        assert!(part.entity(inside_id).unwrap().is_suppressed());
        // A finding reaching outside the region is not spared.
        assert!(!part.entity(crossing_id).unwrap().is_suppressed());
        assert!(!part.entity(outside_id).unwrap().is_suppressed());
        // The decision records the region it covered.
        let recorded = part
            .entity(inside_id)
            .unwrap()
            .audit
            .events()
            .iter()
            .find_map(|event| match &event.kind {
                AuditKind::Manual(manual) => Some(manual.location.clone()),
                _ => None,
            });
        assert_eq!(recorded, Some(region.clone()));

        // A second pass over the same region spares nothing new.
        assert!(
            report
                .document_mut::<Text>()
                .unwrap()
                .suppress_within(&region)
                .is_empty()
        );
    }

    #[test]
    fn entity_anywhere_reaches_a_nested_part_by_id_alone() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id;
        let nested = PartId::new("report.docx").child("word/media/image1.png");
        let mut report = Report::new().insert_part::<Text>(nested.clone(), vec![entity]);

        assert_eq!(
            report.entity_anywhere::<Text>(id).map(|e| e.label.as_str()),
            Some("EMAIL_ADDRESS"),
        );
        report
            .entity_anywhere_mut::<Text>(id)
            .expect("nested entity is reachable by id alone")
            .label = LabelRef::new("RETAGGED");
        assert_eq!(
            report
                .part::<Text>(&nested)
                .unwrap()
                .entity(id)
                .unwrap()
                .label,
            LabelRef::new("RETAGGED"),
        );
        assert!(report.entity_anywhere::<Text>(Uuid::now_v7()).is_none());
    }
}
