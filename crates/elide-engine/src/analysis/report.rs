//! [`Report`]: the detected-but-not-yet-applied entities of a document set,
//! reviewed per part before [`anonymize_with`].
//!
//! Detection (`analyze`) and redaction (`anonymize_with`) are split so a
//! reviewer can inspect the entities in between and say what to change through
//! a [`Review`](crate::Review): suppress a false positive, include a missed
//! one, relabel or relocate a finding, each recorded on the entity's audit
//! trail. A document set's entities span several coordinate systems (each
//! named document's own modality, plus every container part's), so the report
//! keeps them separated: every part's entities are keyed by its [`PartId`]
//! path, each read through a typed accessor. There is no distinct "body", a
//! document's own content is simply its depth-1 part, keyed by the document's
//! name.
//!
//! The report serializes to a part-list view,
//! `{ parts: [ { "id": ["scan-A.docx", "word/media/image1.png"], .. } ] }`,
//! so an external consumer (a review UI) can identify which part each entity
//! belongs to. The path is a segment array (never string-joined); each entity
//! carries its own id, label, location, and confidence.
//!
//! The type-erased storage ([`EntityGroup`]) lives in `group`, and the serde
//! wire view in [`wire`](crate::wire). Each entity carries its
//! own tamper-evident audit trail ([`AuditLog`]) natively, so there is no
//! separate document-level audit type.
//!
//! [`AuditLog`]: elide_core::entity::audit::AuditLog
//! [`anonymize_with`]: crate::Orchestrator::anonymize_with

use std::collections::{BTreeMap, HashMap};

use elide_core::entity::Entity;
use elide_core::entity::audit::AuditHash;
use elide_core::modality::Modality;
use elide_core::redaction::HiddenContent;
use elide_core::{Error, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use super::group::EntityGroup;
use super::part::Part;
use crate::PartId;
use crate::wire::{Restorer, fields, part_fields};

/// The detected entities of a document set, reviewed before apply.
///
/// Returned by [`analyze`] and consumed by [`anonymize_with`]. Every part,
/// each named document's own content (keyed by [`Document::id`]), and every
/// container part within one, is keyed by its [`PartId`] path. Read a part as
/// modality `M` with [`part`]; the view resolves the part and checks its
/// modality once. To reach an entity when only its [`id`] is known, use
/// [`entity_anywhere`].
///
/// A report changes only through a [`Review`](crate::Review) applied by
/// [`Orchestrator::review`], which checks each edit against the
/// orchestrator's [`ReviewPolicy`](crate::ReviewPolicy) and records it on the entities' own audit
/// trails, so the report carries every decision to [`anonymize_with`] and to
/// whoever audits it after.
///
/// A report is **pure entity data**: it carries no live document state, so
/// it can be built from scratch ([`new`] + [`insert_part`]), serialized to a
/// part-list `{ parts: [..] }` view, stored, and read back against its
/// [`Seal`](crate::Seal) with a [`Restorer`]. [`anonymize_with`]
/// re-decodes each part from the container it is applied to, so a rebuilt
/// report redacts just as a freshly-analyzed one does.
///
/// [`analyze`]: crate::Orchestrator::analyze
/// [`anonymize_with`]: crate::Orchestrator::anonymize_with
/// [`Orchestrator::review`]: crate::Orchestrator::review
/// [`part`]: Report::part
/// [`Document::id`]: crate::Document::id
/// [`id`]: elide_core::entity::Entity::id
/// [`entity_anywhere`]: Report::entity_anywhere
/// [`new`]: Report::new
/// [`insert_part`]: Report::insert_part
#[must_use = "a report is the record of what was found and, once applied, of what was redacted"]
pub struct Report {
    /// The report's identity, minted when it is created and kept across
    /// reviews and round trips, so two reports with the same findings (or
    /// none) never share a digest, and a review of one never applies to the
    /// other.
    pub(crate) id: Uuid,
    /// Every part's entities (a `Vec<Entity<M>>` of the part's modality), keyed
    /// by its [`PartId`] path. A named document's own content is its depth-1
    /// part; container parts nest below.
    pub(crate) parts: HashMap<PartId, Box<dyn EntityGroup>>,
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

    /// Every record, ordered by part then item.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&PartId, &HiddenContent, usize)> {
        self.0
            .iter()
            .map(|((part, content), &count)| (part, content, count))
    }
}

/// One item of hidden content stripped from a document: where, what, how many.
///
/// As a [`Report`] records it. Never the stripped value itself, so the report
/// can be shared without shipping what was removed.
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

impl Default for Report {
    fn default() -> Self {
        Self {
            id: Uuid::now_v7(),
            parts: HashMap::new(),
            stripped: StrippedRecord::default(),
        }
    }
}

impl Report {
    /// An empty report with no parts and a fresh identity, to fill in by hand
    /// or by analysis.
    ///
    /// Fill it with [`insert_part`], or let [`analyze`] produce one.
    ///
    /// [`insert_part`]: Self::insert_part
    /// [`analyze`]: crate::Orchestrator::analyze
    pub fn new() -> Self {
        Self::default()
    }

    /// A [`Restorer`] that rebuilds a report without an
    /// [`Orchestrator`].
    ///
    /// It needs none of the analyzers, anonymizers, or codec registry an
    /// [`Orchestrator`] needs to *run* one. Register the modalities the report
    /// may contain, then [`restore`] it against the seal it was written
    /// with:
    ///
    /// ```
    /// use elide_core::modality::text::Text;
    /// use elide_engine::{Report, Seal};
    ///
    /// let written = Report::new();
    /// let seal = Seal::of(&written);
    /// let json = serde_json::to_string(&written)?;
    /// let report: Report = Report::restorer()
    ///     .with_modality::<Text>()
    ///     .restore(&seal, &mut serde_json::Deserializer::from_str(&json))?;
    /// assert_eq!(report.part_ids().count(), 0);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// [`Orchestrator`]: crate::Orchestrator
    /// [`restore`]: Restorer::restore
    #[must_use]
    pub fn restorer() -> Restorer {
        Restorer::new()
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
            .iter()
            .map(|(part, content, count)| StrippedItem {
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

    /// A digest of every entity the report carries: the version a
    /// [`Review`](crate::Review) is made against, and what an unkeyed [`Seal`] of the report
    /// records.
    ///
    /// Each entity's trail [verifies](elide_core::entity::audit::AuditLog::verify)
    /// on its own, but that cannot catch a report rewritten whole: the trail
    /// hashes are unkeyed, so whoever rewrites an entity can recompute its
    /// trail; a trail cut back to an earlier event is intact in its own right;
    /// a label or location changed beside its trail leaves the trail as it
    /// was; and a dropped entity, which would then go unredacted, leaves every
    /// remaining trail intact. The digest covers every part and every
    /// [entity](Entity::digest), its fields and trail alike, so a report read
    /// back against its seal catches any of these. A [review](crate::Review)
    /// changes it.
    ///
    /// Parts are ordered by path and entities by id, so the digest does not
    /// depend on the order the report was built or serialized in. A part with
    /// no entities is left out: dropping one hides nothing, and a restorer
    /// skips one whose modality it does not know. The
    /// [stripped](Self::stripped) section is left out, as it is never read
    /// back.
    ///
    /// [`analyze`]: crate::Orchestrator::analyze
    /// [`anonymize_with`]: crate::Orchestrator::anonymize_with
    /// [`Seal`]: crate::Seal
    #[must_use]
    pub fn digest(&self) -> AuditHash {
        AuditHash::of_fields(fields(self))
    }

    /// The report's identity, a time-ordered UUIDv7 minted when it was
    /// created: what tells two reports with the same findings apart.
    #[must_use]
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// The fields the report's digest covers: its id, then each part with
    /// entities, by path, with its modality and its entities' digests by id.
    pub(crate) fn seal_fields(&self) -> Vec<Vec<u8>> {
        let mut fields = vec![self.id.as_bytes().to_vec()];
        fields.extend(part_fields(
            self.parts
                .iter()
                .filter(|(_, entities)| !entities.is_empty())
                .map(|(id, entities)| {
                    let digests = entities.digests();
                    let body = digests.iter().map(|d| d.as_bytes().to_vec()).collect();
                    (id, entities.modality_name(), body)
                }),
        ));
        fields
    }

    /// Set the entities of the part `id` as modality `P`, replacing any.
    ///
    /// For rebuilding a report from out-of-band entities. [`anonymize_with`]
    /// re-decodes the part `id` from the container and applies these through
    /// the `P` pipeline.
    ///
    /// [`anonymize_with`]: crate::Orchestrator::anonymize_with
    pub fn insert_part<P: Modality>(mut self, id: PartId, entities: Vec<Entity<P>>) -> Self
    where
        P::Location: Serialize + DeserializeOwned,
    {
        self.parts.insert(id, Box::new(entities));
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
        Ok(Part::new(typed::<M>(id, part.as_ref())?))
    }

    /// Find an entity of modality `M` by its [`id`] in any part, read-only.
    ///
    /// For a caller that holds an entity's id but not the part it lives in.
    /// Parts of another modality are skipped.
    ///
    /// [`id`]: elide_core::entity::Entity::id
    pub fn entity_anywhere<M: Modality>(&self, id: Uuid) -> Option<&Entity<M>> {
        self.parts
            .iter()
            .filter_map(|(part_id, part)| typed::<M>(part_id, part.as_ref()).ok())
            .find_map(|entities| entities.iter().find(|entity| entity.id() == id))
    }

    /// The [`PartId`]s of the parts the report carries, paired with each part's
    /// modality name ([`Modality::NAME`], the tag the wire form carries), for a
    /// caller enumerating what's editable. Includes every named document's own
    /// depth-1 part as well as any nested container parts.
    pub fn part_ids(&self) -> impl Iterator<Item = (&PartId, &'static str)> {
        self.parts
            .iter()
            .map(|(id, part)| (id, part.modality_name()))
    }
}

/// The entities of `part` (keyed `id`), as modality `M`.
fn typed<'p, M: Modality>(id: &PartId, part: &'p dyn EntityGroup) -> Result<&'p [Entity<M>]> {
    part.as_any()
        .downcast_ref::<Vec<Entity<M>>>()
        .map(Vec::as_slice)
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
    use elide_core::modality::NoArtifact;
    use elide_core::modality::text::{Text, TextLocation};

    use super::super::test_support::{doc, text_entity, text_entity_at as at};
    use super::*;

    /// The digest moves with anything an entity says, on its trail or beside
    /// it, and with an entity dropped; it does not move with the order the
    /// entities were built in, or with a part holding none.
    #[test]
    fn the_digest_covers_every_entity_and_field() {
        let kept = at("EMAIL_ADDRESS", 0..5);
        let other = at("PERSON_NAME", 6..9);
        let id = Uuid::now_v7();
        let report = |entities: Vec<Entity<Text>>| {
            let mut report = Report::new()
                .insert_part::<Text>(doc(), entities)
                .insert_part::<Text>(doc().child("empty"), Vec::new());
            report.id = id;
            report
        };
        let written = report(vec![kept.clone(), other.clone()]);
        let digest = written.digest();
        assert_eq!(report(vec![other.clone(), kept.clone()]).digest(), digest);

        // A label or location changed beside the trail, on the wire.
        let registry = Restorer::new().with_modality::<Text>();
        let wire = serde_json::to_value(&written).unwrap();
        let moved_to = serde_json::to_value(TextLocation::inline(0..2)).unwrap();
        for (field, value) in [
            ("label", serde_json::json!("RETAGGED")),
            ("location", moved_to),
        ] {
            let mut tampered = wire.clone();
            for entity in tampered["parts"][0]["entities"].as_array_mut().unwrap() {
                if entity["id"] == serde_json::json!(kept.id()) {
                    entity[field] = value.clone();
                }
            }
            let read: Report = registry.restore_unsealed(tampered).unwrap();
            assert!(read.entity_anywhere::<Text>(kept.id()).is_some());
            assert_ne!(read.digest(), digest, "a changed {field} moves the digest");
        }

        assert_ne!(report(vec![kept.clone()]).digest(), digest);

        let mut without_empty = Report::new().insert_part::<Text>(doc(), vec![kept, other]);
        without_empty.id = id;
        assert_eq!(without_empty.digest(), digest);

        // Another report with the same findings has another identity.
        assert_ne!(
            Report::new()
                .insert_part::<Text>(doc(), vec![at("EMAIL_ADDRESS", 0..5)])
                .digest(),
            Report::new()
                .insert_part::<Text>(doc(), vec![at("EMAIL_ADDRESS", 0..5)])
                .digest(),
        );
    }

    #[test]
    fn a_part_is_read_by_its_id() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id();
        let part = doc().child("word/media/image1.png");
        let report = Report::new().insert_part::<Text>(part.clone(), vec![entity]);

        let read = report.part::<Text>(&part).unwrap();
        assert_eq!(read.entities().len(), 1);
        assert_eq!(
            *read.entity(id).unwrap().label(),
            LabelRef::new("EMAIL_ADDRESS")
        );
    }

    #[test]
    fn an_unknown_part_or_another_modality_is_not_found() {
        let report = Report::new().insert_part::<Text>(doc(), vec![text_entity("X")]);
        let missing = report.part::<Text>(&PartId::leaf("missing")).unwrap_err();
        assert_eq!(missing.kind(), ErrorKind::NotFound);

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        struct Other;
        impl Modality for Other {
            type Artifact = NoArtifact;
            type Data = <Text as Modality>::Data;
            type Location = <Text as Modality>::Location;
            type Replacement = <Text as Modality>::Replacement;

            const NAME: &'static str = "other";
        }
        assert_eq!(
            report.part::<Other>(&doc()).unwrap_err().kind(),
            ErrorKind::NotFound,
        );
    }

    #[test]
    fn entity_anywhere_reaches_a_nested_part_by_id_alone() {
        let entity = text_entity("EMAIL_ADDRESS");
        let id = entity.id();
        let nested = PartId::leaf("report.docx").child("word/media/image1.png");
        let report = Report::new().insert_part::<Text>(nested, vec![entity]);

        assert_eq!(
            report
                .entity_anywhere::<Text>(id)
                .map(|e| e.label().as_str()),
            Some("EMAIL_ADDRESS"),
        );
        assert!(report.entity_anywhere::<Text>(Uuid::now_v7()).is_none());
    }
}
