//! [`Part`] and [`PartMut`]: one part of a [`Report`], typed to its modality,
//! to read and to edit.
//!
//! [`Report`]: super::Report

use elide_core::entity::audit::{Attribution, AuditEvent, Manual, ManualIntent};
use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::{Modality, ModalityLocation, Overlap};
use elide_core::primitive::Confidence;
use uuid::Uuid;

/// One part of a report, typed to its modality, for reading its entities.
///
/// Returned by [`Report::part`] and [`Report::document`].
///
/// [`Report::part`]: super::Report::part
/// [`Report::document`]: super::Report::document
#[derive(Debug, Clone, Copy)]
pub struct Part<'r, M: Modality> {
    entities: &'r [Entity<M>],
}

impl<'r, M: Modality> Part<'r, M> {
    pub(crate) fn new(entities: &'r [Entity<M>]) -> Self {
        Self { entities }
    }

    /// Every entity of the part, suppressed or not.
    #[must_use]
    pub fn entities(&self) -> &'r [Entity<M>] {
        self.entities
    }

    /// The entity `id`, if the part holds it.
    #[must_use]
    pub fn entity(&self, id: Uuid) -> Option<&'r Entity<M>> {
        self.entities.iter().find(|entity| entity.id == id)
    }

    /// The entities a reviewer suppressed, which redaction leaves as they are.
    pub fn suppressed(&self) -> impl Iterator<Item = &'r Entity<M>> {
        self.entities.iter().filter(|entity| entity.is_suppressed())
    }

    /// The entities no reviewer suppressed, which redaction considers. Whether
    /// one is hidden is still the anonymizer's rules' decision.
    pub fn unsuppressed(&self) -> impl Iterator<Item = &'r Entity<M>> {
        self.entities
            .iter()
            .filter(|entity| !entity.is_suppressed())
    }
}

/// One part of a report, typed to its modality, for editing its entities.
///
/// Returned by [`Report::part_mut`] and [`Report::document_mut`]. Every edit
/// made through it is recorded on the entities it touches as a `Manual` audit
/// event, attributed to [`by`](Self::by) and [`because`](Self::because) when
/// set, so a reviewer's decision is never mistaken for a detection.
///
/// [`Report::part_mut`]: super::Report::part_mut
/// [`Report::document_mut`]: super::Report::document_mut
#[derive(Debug)]
pub struct PartMut<'r, M: Modality> {
    entities: &'r mut Vec<Entity<M>>,
    actor: Option<String>,
    attribution: Option<Attribution>,
}

impl<'r, M: Modality> PartMut<'r, M> {
    pub(crate) fn new(entities: &'r mut Vec<Entity<M>>) -> Self {
        Self {
            entities,
            actor: None,
            attribution: None,
        }
    }

    /// Attribute the edits made through this view to `actor`.
    #[must_use]
    pub fn by(mut self, actor: impl Into<String>) -> Self {
        self.actor = Some(actor.into());
        self
    }

    /// Record `attribution` as the reason for the edits made through this view.
    #[must_use]
    pub fn because(mut self, attribution: impl Into<Attribution>) -> Self {
        self.attribution = Some(attribution.into());
        self
    }

    /// Read the part through this view.
    #[must_use]
    pub fn as_part(&self) -> Part<'_, M> {
        Part::new(self.entities)
    }

    /// Add an entity of `label` at `location`, one detection missed, and return
    /// its id.
    ///
    /// A reviewer's assertion is certain, so it carries [`Confidence::MAX`].
    pub fn include(&mut self, label: impl Into<LabelRef>, location: M::Location) -> Uuid {
        let flag = decision(
            ManualIntent::Flag,
            location.clone(),
            self.attribution.clone(),
        );
        let event = AuditEvent::manual(self.source(), Confidence::MAX, flag);
        let entity = Entity::new(label, location).with_event(event);
        let id = entity.id;
        self.entities.push(entity);
        id
    }

    /// Give the entity `id` the label `label`, recording the amendment. Returns
    /// `false` when the part holds no such entity.
    pub fn relabel(&mut self, id: Uuid, label: impl Into<LabelRef>) -> bool {
        let (actor, attribution) = (self.actor.clone(), self.attribution.clone());
        let Some(entity) = self.entity_mut(id) else {
            return false;
        };
        entity.label = label.into();
        let amend = decision(ManualIntent::Amend, entity.location.clone(), attribution);
        entity.record_manual(amend, actor.as_deref())
    }

    /// Move the entity `id` to `location`, recording the amendment there.
    /// Returns `false` when the part holds no such entity.
    pub fn relocate(&mut self, id: Uuid, location: M::Location) -> bool {
        let (actor, attribution) = (self.actor.clone(), self.attribution.clone());
        let Some(entity) = self.entity_mut(id) else {
            return false;
        };
        entity.location = location.clone();
        let amend = decision(ManualIntent::Amend, location, attribution);
        entity.record_manual(amend, actor.as_deref())
    }

    /// Leave the entity `id` unredacted. Returns `false` when the part holds no
    /// such entity or it is already suppressed.
    pub fn suppress(&mut self, id: Uuid) -> bool {
        let (actor, attribution) = (self.actor.clone(), self.attribution.clone());
        let Some(entity) = self.entity_mut(id) else {
            return false;
        };
        let suppress = decision(ManualIntent::Suppress, entity.location.clone(), attribution);
        entity.record_manual(suppress, actor.as_deref())
    }

    /// Leave every entity `region` contains unredacted, and return the ids this
    /// call suppressed.
    ///
    /// An entity reaching outside `region` is kept, so nothing outside it is
    /// spared. Each suppression records `region` as where the decision applies.
    /// The suppressions are on the report already; the ids are for a caller that
    /// shows what the region cleared.
    pub fn suppress_within(&mut self, region: &M::Location) -> Vec<Uuid> {
        let suppress = decision(
            ManualIntent::Suppress,
            region.clone(),
            self.attribution.clone(),
        );
        let actor = self.actor.clone();
        self.entities
            .iter_mut()
            .filter(|entity| region.overlap(&entity.location) == Overlap::Contains)
            .filter_map(|entity| {
                entity
                    .record_manual(suppress.clone(), actor.as_deref())
                    .then_some(entity.id)
            })
            .collect()
    }

    /// The entity `id`, if the part holds it.
    fn entity_mut(&mut self, id: Uuid) -> Option<&mut Entity<M>> {
        self.entities.iter_mut().find(|entity| entity.id == id)
    }

    /// The audit source of this view's edits: the actor, or `"manual"`.
    fn source(&self) -> String {
        self.actor.clone().unwrap_or_else(|| "manual".to_owned())
    }
}

/// A reviewer's `Manual` decision of `intent`, applying at `location` and
/// carrying `attribution`.
fn decision<M: Modality>(
    intent: ManualIntent,
    location: M::Location,
    attribution: Option<Attribution>,
) -> Manual<M> {
    let manual = Manual::new(intent, location);
    match attribution {
        Some(attribution) => manual.with_attribution(attribution),
        None => manual,
    }
}
