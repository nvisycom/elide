//! [`EditGroup`]: one part's edits, a `Vec<Edit<M>>` behind one type, each
//! checked against a policy and applied in turn.

use elide_core::entity::audit::Manual;
use elide_core::entity::{Birth, Entity, Include};
use elide_core::modality::{Modality, ModalityLocation};
use elide_core::primitive::Confidence;
use elide_core::{Error, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use super::edit::{Change, Edit};
use super::policy::{Narrowed, ProposedEdit, ReviewPolicy};
use crate::PartId;
use crate::analysis::{EntityGroup, ErasedGroup, GroupItem};

/// A group of one modality's review edits, a `Vec<Edit<M>>` behind one type,
/// held erased like a report's entities.
pub(crate) trait EditGroup: ErasedGroup {
    /// Each edit's fields, in order, for the review's digest.
    fn fields(&self, out: &mut Vec<Vec<u8>>);
    /// The ids the group's includes give their entities.
    fn included(&self) -> Vec<Uuid>;
    /// An empty entity group of this modality, for a part the report does not
    /// hold yet, which an include creates.
    fn empty_entities(&self) -> Box<dyn EntityGroup>;
    /// The part's `entities` with every edit applied in order, each made by
    /// the `reviewer` and checked against their policy as the edits before it
    /// left them. `entities` is left as it is, so a refused edit changes
    /// nothing.
    fn apply(
        &self,
        part: &PartId,
        entities: &dyn EntityGroup,
        reviewer: Reviewer<'_>,
    ) -> Result<Box<dyn EntityGroup>>;
}

/// Who applies a review, and the policy their edits are checked against.
#[derive(Clone, Copy)]
pub(crate) struct Reviewer<'a> {
    /// The policy each edit is checked against.
    pub(crate) policy: &'a dyn ReviewPolicy,
    /// Who made the edits.
    pub(crate) by: &'a str,
}

impl<M: Modality> GroupItem for Edit<M> {
    type Modality = M;
}

impl<M: Modality> EditGroup for Vec<Edit<M>>
where
    M::Location: Serialize + DeserializeOwned,
{
    fn fields(&self, out: &mut Vec<Vec<u8>>) {
        for edit in self {
            edit.fields(out);
        }
    }

    fn included(&self) -> Vec<Uuid> {
        self.iter()
            .filter_map(|edit| match edit.change {
                Change::Include { entity, .. } => Some(entity),
                _ => None,
            })
            .collect()
    }

    fn empty_entities(&self) -> Box<dyn EntityGroup> {
        Box::new(Vec::<Entity<M>>::new())
    }

    fn apply(
        &self,
        part: &PartId,
        entities: &dyn EntityGroup,
        reviewer: Reviewer<'_>,
    ) -> Result<Box<dyn EntityGroup>> {
        let mut entities = entities
            .as_any()
            .downcast_ref::<Vec<Entity<M>>>()
            .ok_or_else(|| {
                Error::not_found(format!(
                    "the review edits part `{part}` as {}, which the report holds as another modality",
                    M::NAME
                ))
            })?
            .clone();
        let review = PartReview { part, reviewer };
        for edit in self {
            review.step(&mut entities, edit)?;
        }
        Ok(Box::new(entities))
    }
}

/// One part of a review being applied, by its reviewer.
struct PartReview<'a> {
    part: &'a PartId,
    reviewer: Reviewer<'a>,
}

impl PartReview<'_> {
    /// Check `edit` against the reviewer's policy, and apply it to the part's
    /// `entities` once accepted.
    fn step<M: Modality>(&self, entities: &mut Vec<Entity<M>>, edit: &Edit<M>) -> Result<()> {
        let by = self.reviewer.by;
        let targets = targets(self.part, entities, &edit.change)?;
        let narrowed = narrowed(entities, &targets, &edit.change);
        let decision = edit.change.intent();
        self.reviewer.policy.check(&ProposedEdit {
            part: self.part,
            decision: &decision,
            narrowed: &narrowed,
            by,
            because: edit.because.as_ref(),
        })?;

        let manual = |location: M::Location| Manual {
            intent: decision.clone(),
            location,
            attribution: edit.because.clone(),
        };
        match &edit.change {
            Change::Include {
                entity: id,
                label,
                location,
            } => {
                let included = Include {
                    label: label.clone(),
                    location: location.clone(),
                    attribution: edit.because.clone(),
                };
                let birth = Birth::new(by.to_owned(), Confidence::MAX, included).with_id(*id);
                entities.push(Entity::new(birth));
            }
            change => {
                for i in targets {
                    let at = match change {
                        Change::SuppressWithin { region } => region.clone(),
                        Change::Relocate { location, .. } => location.clone(),
                        _ => entities[i].location().clone(),
                    };
                    entities[i].record_manual(manual(at), by)?;
                }
            }
        }
        Ok(())
    }
}

/// The entities an edit narrows: every one it touches, and every one in an
/// overlap cluster with what it touches, before the edit or after.
///
/// An include or a confirmation only adds to what is hidden, so it narrows
/// nothing. A relocation can narrow even into a region containing the old
/// one, as a rule may choose its operator by where an entity sits. Entities
/// that overlap, directly or through others, are redacted as one span under
/// the safest operator among them, so an edit to one can leave the rest of
/// its cluster under a leakier operator, or none: they are narrowed too. An
/// entity already suppressed is not redacted, so no edit narrows it, nor
/// links a cluster.
fn narrowed<M: Modality>(
    entities: &[Entity<M>],
    targets: &[usize],
    change: &Change<M>,
) -> Vec<Narrowed> {
    if !change.can_narrow() {
        return Vec::new();
    }
    let live: Vec<bool> = entities
        .iter()
        .map(|entity| !entity.is_suppressed())
        .collect();
    let mut cluster = vec![false; entities.len()];
    let mut seeds: Vec<usize> = targets.iter().copied().filter(|&i| live[i]).collect();
    // A relocation joins the cluster at its destination only when what moves
    // is redacted: a suppressed entity moved anywhere changes no redaction.
    if let Change::Relocate { location, .. } = change
        && !seeds.is_empty()
    {
        seeds.extend(
            (0..entities.len()).filter(|&i| live[i] && location.overlaps(entities[i].location())),
        );
    }
    // Grow the cluster through overlaps: each member, once in, brings in the
    // live entities it overlaps.
    let mut pending = Vec::new();
    for i in seeds {
        if !cluster[i] {
            cluster[i] = true;
            pending.push(i);
        }
    }
    while let Some(j) = pending.pop() {
        for i in 0..entities.len() {
            if !cluster[i] && live[i] && entities[j].location().overlaps(entities[i].location()) {
                cluster[i] = true;
                pending.push(i);
            }
        }
    }
    entities
        .iter()
        .enumerate()
        .filter(|&(i, _)| cluster[i])
        .map(|(_, entity)| Narrowed {
            id: entity.id(),
            label: entity.label().clone(),
        })
        .collect()
}

/// The indices of the entities of `part` that `change` touches: none for an
/// include, every one a region contains for a region suppression, and the one
/// it names otherwise.
///
/// # Errors
///
/// When the change names an entity the part does not hold.
fn targets<M: Modality>(
    part: &PartId,
    entities: &[Entity<M>],
    change: &Change<M>,
) -> Result<Vec<usize>> {
    let position = |id| entities.iter().position(|entity| entity.id() == id);
    match change {
        Change::Include { .. } => Ok(Vec::new()),
        Change::SuppressWithin { region } => Ok((0..entities.len())
            .filter(|&i| region.covers(entities[i].location()))
            .collect()),
        Change::Suppress { entity: id }
        | Change::Confirm { entity: id }
        | Change::Relabel { entity: id, .. }
        | Change::Relocate { entity: id, .. } => position(*id)
            .map(|i| vec![i])
            .ok_or_else(|| Error::not_found(format!("part `{part}` holds no entity `{id}`"))),
    }
}
